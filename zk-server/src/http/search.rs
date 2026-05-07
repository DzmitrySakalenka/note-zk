//! Search-scoped HTTP routes.

use std::{collections::HashMap, io};

use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use zk_ai::{
    AiError,
    provider::embed_one,
    service::semantic_search::{
        EmbeddingsReader, SearchHit, TopHit as AiTopHit, build_search_hits,
    },
};
use zk_domain::{AuthScope, Capability, ChunkId, EmbeddingModelId, EmbeddingVector};

use crate::{
    ServerDb, ServerError,
    auth::AuthenticatedDevice,
    http::{ApiError, run_blocking},
    repo::EmbeddingsRepo,
    state::ServerState,
};

const DEFAULT_TOP_K: usize = 10;
const MAX_TOP_K: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticSearchRequest {
    query: String,
    top_k: Option<usize>,
}

#[derive(Debug, Serialize)]
struct SemanticSearchResponse {
    hits: Vec<SearchHitDto>,
    query_dim: usize,
    model: String,
}

#[derive(Debug, Serialize)]
struct SearchHitDto {
    note_id: String,
    chunk_id: String,
    similarity: f32,
    snippet: String,
}

/// Returns AI search routes without state binding.
pub(crate) fn search_routes() -> Router<ServerState> {
    Router::new().route("/search/semantic", post(semantic_search))
}

/// Embeds one query and scans stored chunk embeddings with brute-force cosine.
///
/// V1 intentionally performs one linear scan over stored embeddings for the
/// selected model. This is simple and predictable, but query latency grows with
/// the number of stored chunks until a future vector index lands.
async fn semantic_search(
    State(state): State<ServerState>,
    device: AuthenticatedDevice,
    Json(request): Json<SemanticSearchRequest>,
) -> Result<Json<SemanticSearchResponse>, ApiError> {
    require_search_scope(&device)?;

    if request.query.trim().is_empty() {
        return Err(ApiError::from(ServerError::InvalidRequest(
            "query must not be empty".to_string(),
        )));
    }

    let selector = state
        .ai_selector()
        .ok_or_else(|| ApiError::from(ServerError::NotFound("not found".to_string())))?;
    let provider = selector
        .select_embeddings(Capability::SemanticSearch)
        .map_err(ai_error_to_api_error)?;
    let capability = provider.capability().clone();
    let top_k = request.top_k.unwrap_or(DEFAULT_TOP_K).min(MAX_TOP_K);
    let model = capability.model.clone();
    let mut query_vector = embed_one(provider.as_ref(), request.query, model.clone())
        .await
        .map_err(ai_error_to_api_error)?;
    query_vector.normalize_in_place();
    let db = state.db_handle();
    let hits = run_blocking("semantic search", move || {
        semantic_search_blocking(db.as_ref(), &query_vector, top_k, &model)
    })
    .await?;

    Ok(Json(SemanticSearchResponse {
        hits: hits.into_iter().map(search_hit_dto).collect(),
        query_dim: capability.dim,
        model: capability.model.to_string(),
    }))
}

fn require_search_scope(device: &AuthenticatedDevice) -> Result<(), ApiError> {
    if device.scopes.contains(&AuthScope::Sync) || device.scopes.contains(&AuthScope::Admin) {
        return Ok(());
    }

    Err(ApiError::from(ServerError::Forbidden("sync".to_string())))
}

fn search_hit_dto(hit: SearchHit) -> SearchHitDto {
    SearchHitDto {
        note_id: hit.note_id.to_string(),
        chunk_id: hit.chunk_id.to_string(),
        similarity: hit.similarity,
        snippet: hit.snippet,
    }
}

fn ai_error_to_api_error(error: AiError) -> ApiError {
    match error {
        AiError::ProviderUnavailable(_) => ApiError::from(ServerError::ServiceUnavailable(
            "ai service unavailable".to_string(),
        )),
        AiError::ProviderRequest(_) => ApiError::from(ServerError::BadGateway(
            "ai provider request failed".to_string(),
        )),
        AiError::InvalidEmbeddingBlob { len } => {
            ApiError::from(ServerError::InvalidEmbeddingBlob { len })
        }
        other => ApiError::from(ServerError::Internal(Box::new(io::Error::other(
            other.to_string(),
        )))),
    }
}

fn semantic_search_blocking(
    db: &ServerDb,
    query_vector: &EmbeddingVector,
    top_k: usize,
    model: &EmbeddingModelId,
) -> crate::ServerResult<Vec<SearchHit>> {
    db.with_conn(|connection| {
        let repo = EmbeddingsRepo::new(connection);
        let top_hits = repo
            .top_k_cosine(query_vector, top_k, model)?
            .into_iter()
            .map(|hit| AiTopHit {
                note_id: hit.note_id,
                chunk_id: hit.chunk_id,
                similarity: hit.similarity,
            })
            .collect::<Vec<_>>();
        if top_hits.is_empty() {
            return Ok(Vec::new());
        }

        let chunk_ids = top_hits
            .iter()
            .map(|hit| hit.chunk_id.clone())
            .collect::<Vec<ChunkId>>();
        let text_by_chunk = EmbeddingsReader::list_chunk_text(&repo, &chunk_ids)
            .map_err(|error| ai_to_server_error(&error))?
            .into_iter()
            .collect::<HashMap<_, _>>();

        build_search_hits(top_hits, &text_by_chunk).map_err(|error| ai_to_server_error(&error))
    })
}

fn ai_to_server_error(error: &AiError) -> ServerError {
    match error {
        AiError::InvalidEmbeddingBlob { len } => ServerError::InvalidEmbeddingBlob { len: *len },
        other => ServerError::Internal(Box::new(io::Error::other(other.to_string()))),
    }
}
