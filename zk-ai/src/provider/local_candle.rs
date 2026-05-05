//! Candle-backed local embeddings provider.

use std::{
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};
use zk_domain::{EmbeddingModelId, EmbeddingVector, ProviderId};

use super::{EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse};
use crate::{AiError, AiResult};

const CONFIG_FILE: &str = "config.json";
const DEFAULT_MAX_BATCH_SIZE: usize = 32;
const DEFAULT_MAX_SEQ_LEN: usize = 512;
const PROVIDER_ID: &str = "local-candle";
const PYTORCH_WEIGHTS_FILE: &str = "pytorch_model.bin";
const SAFETENSORS_WEIGHTS_FILE: &str = "model.safetensors";
const TOKENIZER_FILE: &str = "tokenizer.json";

/// Device requested for local Candle embeddings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDevice {
    /// Run inference on CPU.
    Cpu,
    /// Run inference on Apple Metal, requiring the `provider-local-metal` feature.
    Metal,
    /// Run inference on CUDA, requiring the `provider-local-cuda` feature.
    Cuda,
}

/// Configuration for the Candle-backed local embeddings provider.
#[derive(Debug, Clone)]
pub struct LocalEmbeddingsConfig {
    /// Directory containing `tokenizer.json`, `config.json`, and model weights.
    pub model_dir: PathBuf,
    /// Provider default model advertised in capability metadata.
    pub model_id: EmbeddingModelId,
    /// Maximum token sequence length for truncation.
    pub max_seq_len: Option<usize>,
    /// Maximum texts sent in one provider call.
    pub max_batch_size: Option<usize>,
    /// Candle device requested for inference.
    pub device: LocalDevice,
}

/// Candle-backed local embeddings provider.
#[derive(Clone)]
pub struct LocalEmbeddingsProvider {
    id: ProviderId,
    capability: EmbeddingsCapability,
    tokenizer: Tokenizer,
    encoder: Arc<dyn LocalEncoder + Send + Sync>,
    max_seq_len: usize,
}

impl fmt::Debug for LocalEmbeddingsProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalEmbeddingsProvider")
            .field("id", &self.id)
            .field("capability", &self.capability)
            .field("max_seq_len", &self.max_seq_len)
            .finish_non_exhaustive()
    }
}

impl LocalEmbeddingsProvider {
    /// Creates a local embeddings provider from a model directory.
    ///
    /// # Errors
    ///
    /// Returns [`AiError::InvalidConfig`] for invalid limits or device-feature
    /// requests. Returns [`AiError::ProviderUnavailable`] when required model
    /// files are missing or Candle cannot load the model.
    pub fn from_config(config: LocalEmbeddingsConfig) -> AiResult<Self> {
        let max_seq_len =
            positive_or_default(config.max_seq_len, DEFAULT_MAX_SEQ_LEN, "max_seq_len")?;
        let max_batch_size = positive_or_default(
            config.max_batch_size,
            DEFAULT_MAX_BATCH_SIZE,
            "max_batch_size",
        )?;
        let tokenizer_path = required_file(&config.model_dir, TOKENIZER_FILE)?;
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|error| {
            AiError::ProviderUnavailable(format!(
                "load tokenizer {}: {error}",
                tokenizer_path.display()
            ))
        })?;
        let encoder = Arc::new(BertLocalEncoder::from_model_dir(
            &config.model_dir,
            config.device,
        )?);
        let dim = probe_dimension(encoder.as_ref(), tokenizer.clone(), max_seq_len)?;
        let id = PROVIDER_ID
            .parse()
            .map_err(|error| AiError::Internal(format!("parse hardcoded provider id: {error}")))?;

        Ok(Self {
            id,
            capability: EmbeddingsCapability {
                model: config.model_id,
                dim,
                max_batch_size,
                max_input_tokens: u32::try_from(max_seq_len).map_err(|error| {
                    AiError::InvalidConfig(format!("max_seq_len is too large: {error}"))
                })?,
            },
            tokenizer,
            encoder,
            max_seq_len,
        })
    }
}

#[async_trait]
impl EmbeddingsProvider for LocalEmbeddingsProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capability(&self) -> &EmbeddingsCapability {
        &self.capability
    }

    async fn embed_batch(&self, request: EmbeddingsRequest) -> AiResult<EmbeddingsResponse> {
        if request.texts.len() > self.capability.max_batch_size {
            return Err(AiError::InvalidConfig(
                "embedding batch size exceeds provider capability".to_string(),
            ));
        }

        if request.texts.is_empty() {
            return Ok(EmbeddingsResponse {
                vectors: Vec::new(),
            });
        }

        let EmbeddingsRequest { texts, model } = request;
        let tokenizer = self.tokenizer.clone();
        let encoder = Arc::clone(&self.encoder);
        let max_seq_len = self.max_seq_len;
        let values =
            tokio::task::spawn_blocking(move || encoder.encode(tokenizer, texts, max_seq_len))
                .await
                .map_err(|error| {
                    AiError::Internal(format!("join local embeddings task: {error}"))
                })??;
        let vectors = values
            .into_iter()
            .map(|values| EmbeddingVector {
                values,
                model: model.clone(),
            })
            .collect();

        Ok(EmbeddingsResponse { vectors })
    }
}

trait LocalEncoder {
    fn encode(
        &self,
        tokenizer: Tokenizer,
        texts: Vec<String>,
        max_seq_len: usize,
    ) -> AiResult<Vec<Vec<f32>>>;
}

struct BertLocalEncoder {
    model: BertModel,
    device: Device,
}

impl BertLocalEncoder {
    fn from_model_dir(model_dir: &Path, local_device: LocalDevice) -> AiResult<Self> {
        let config_path = required_file(model_dir, CONFIG_FILE)?;
        let config = fs::read_to_string(&config_path).map_err(|error| {
            AiError::ProviderUnavailable(format!(
                "read BERT config {}: {error}",
                config_path.display()
            ))
        })?;
        let config = serde_json::from_str::<Config>(&config).map_err(|error| {
            AiError::ProviderUnavailable(format!(
                "parse BERT config {}: {error}",
                config_path.display()
            ))
        })?;
        let weights = find_weights(model_dir)?;
        let device = local_device.into_candle_device()?;
        let model = load_bert_model(&weights, &device, &config)?;

        Ok(Self { model, device })
    }
}

impl LocalEncoder for BertLocalEncoder {
    fn encode(
        &self,
        mut tokenizer: Tokenizer,
        texts: Vec<String>,
        max_seq_len: usize,
    ) -> AiResult<Vec<Vec<f32>>> {
        configure_tokenizer(&mut tokenizer, max_seq_len)?;
        let encodings = tokenizer.encode_batch(texts, true).map_err(|error| {
            AiError::ProviderRequest(format!("tokenize local embeddings batch: {error}"))
        })?;

        if encodings.is_empty() {
            return Ok(Vec::new());
        }

        let token_ids = encodings
            .iter()
            .map(|encoding| tensor_from_u32(encoding.get_ids(), &self.device))
            .collect::<AiResult<Vec<_>>>()?;
        let attention_mask = encodings
            .iter()
            .map(|encoding| tensor_from_u32(encoding.get_attention_mask(), &self.device))
            .collect::<AiResult<Vec<_>>>()?;
        let token_ids = Tensor::stack(&token_ids, 0).map_err(candle_request_error)?;
        let attention_mask = Tensor::stack(&attention_mask, 0).map_err(candle_request_error)?;
        let token_type_ids = token_ids.zeros_like().map_err(candle_request_error)?;
        let token_embeddings = self
            .model
            .forward(&token_ids, &token_type_ids, Some(&attention_mask))
            .map_err(candle_request_error)?;
        let pooled = mean_pool(&token_embeddings, &attention_mask)?;
        let mut vectors = pooled.to_vec2::<f32>().map_err(candle_request_error)?;

        for values in &mut vectors {
            normalize_values(values);
        }

        Ok(vectors)
    }
}

enum WeightFile {
    Safetensors(PathBuf),
    Pytorch(PathBuf),
}

impl LocalDevice {
    fn into_candle_device(self) -> AiResult<Device> {
        match self {
            Self::Cpu => Ok(Device::Cpu),
            Self::Metal => metal_device(),
            Self::Cuda => cuda_device(),
        }
    }
}

#[cfg(feature = "provider-local-metal")]
fn metal_device() -> AiResult<Device> {
    Device::new_metal(0).map_err(|error| {
        AiError::ProviderUnavailable(format!("initialize local Candle Metal device: {error}"))
    })
}

#[cfg(not(feature = "provider-local-metal"))]
fn metal_device() -> AiResult<Device> {
    Err(AiError::InvalidConfig(
        "local Candle Metal device requires provider-local-metal feature".to_string(),
    ))
}

#[cfg(feature = "provider-local-cuda")]
fn cuda_device() -> AiResult<Device> {
    Device::new_cuda(0).map_err(|error| {
        AiError::ProviderUnavailable(format!("initialize local Candle CUDA device: {error}"))
    })
}

#[cfg(not(feature = "provider-local-cuda"))]
fn cuda_device() -> AiResult<Device> {
    Err(AiError::InvalidConfig(
        "local Candle CUDA device requires provider-local-cuda feature".to_string(),
    ))
}

fn positive_or_default(value: Option<usize>, default: usize, field: &str) -> AiResult<usize> {
    let value = value.unwrap_or(default);
    if value == 0 {
        return Err(AiError::InvalidConfig(format!(
            "{field} must be greater than zero"
        )));
    }

    Ok(value)
}

fn required_file(model_dir: &Path, file_name: &str) -> AiResult<PathBuf> {
    let path = model_dir.join(file_name);
    if path.is_file() {
        return Ok(path);
    }

    Err(AiError::ProviderUnavailable(format!(
        "missing local model file: {}",
        path.display()
    )))
}

fn find_weights(model_dir: &Path) -> AiResult<WeightFile> {
    let safetensors = model_dir.join(SAFETENSORS_WEIGHTS_FILE);
    if safetensors.is_file() {
        return Ok(WeightFile::Safetensors(safetensors));
    }

    let pytorch = model_dir.join(PYTORCH_WEIGHTS_FILE);
    if pytorch.is_file() {
        return Ok(WeightFile::Pytorch(pytorch));
    }

    Err(AiError::ProviderUnavailable(format!(
        "missing local model weights: expected {} or {}",
        safetensors.display(),
        pytorch.display()
    )))
}

fn load_bert_model(weights: &WeightFile, device: &Device, config: &Config) -> AiResult<BertModel> {
    let var_builder = match weights {
        WeightFile::Safetensors(path) => {
            // SAFETY: Candle uses memory mapping for safetensors loading; the
            // path was validated as a regular file and is only borrowed during
            // model construction.
            unsafe { VarBuilder::from_mmaped_safetensors(&[path], DTYPE, device) }
        }
        WeightFile::Pytorch(path) => VarBuilder::from_pth(path, DTYPE, device),
    }
    .map_err(|error| AiError::ProviderUnavailable(format!("load local BERT weights: {error}")))?;

    BertModel::load(var_builder, config)
        .map_err(|error| AiError::ProviderUnavailable(format!("load local BERT model: {error}")))
}

fn configure_tokenizer(tokenizer: &mut Tokenizer, max_seq_len: usize) -> AiResult<()> {
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: max_seq_len,
            ..Default::default()
        }))
        .map_err(|error| {
            AiError::InvalidConfig(format!("configure tokenizer truncation: {error}"))
        })?;
    tokenizer.with_padding(Some(PaddingParams {
        strategy: PaddingStrategy::BatchLongest,
        ..Default::default()
    }));

    Ok(())
}

fn tensor_from_u32(values: &[u32], device: &Device) -> AiResult<Tensor> {
    Tensor::new(values, device).map_err(candle_request_error)
}

fn mean_pool(token_embeddings: &Tensor, attention_mask: &Tensor) -> AiResult<Tensor> {
    let attention_mask = attention_mask
        .to_dtype(DTYPE)
        .map_err(candle_request_error)?
        .unsqueeze(2)
        .map_err(candle_request_error)?;
    let sum_mask = attention_mask.sum(1).map_err(candle_request_error)?;
    let masked_embeddings = token_embeddings
        .broadcast_mul(&attention_mask)
        .map_err(candle_request_error)?;
    let summed_embeddings = masked_embeddings.sum(1).map_err(candle_request_error)?;

    summed_embeddings
        .broadcast_div(&sum_mask)
        .map_err(candle_request_error)
}

fn probe_dimension(
    encoder: &(dyn LocalEncoder + Send + Sync),
    tokenizer: Tokenizer,
    max_seq_len: usize,
) -> AiResult<usize> {
    let vectors = encoder.encode(tokenizer, vec!["test".to_string()], max_seq_len)?;
    let vector = vectors.first().ok_or_else(|| {
        AiError::Internal("local embeddings probe returned no vector".to_string())
    })?;

    Ok(vector.len())
}

fn normalize_values(values: &mut [f32]) {
    let squared_norm = values
        .iter()
        .fold(0.0_f32, |sum, value| sum + (value * value));

    if squared_norm == 0.0 {
        return;
    }

    let inverse_norm = squared_norm.sqrt().recip();
    for value in values {
        *value *= inverse_norm;
    }
}

fn candle_request_error(error: impl std::fmt::Display) -> AiError {
    AiError::ProviderRequest(format!("run local BERT embeddings: {error}"))
}
