//! Search-scoped HTTP routes.

use std::{io, sync::Arc};

use axum::{Json, Router, extract::State, routing::post};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use zk_ai::{
    AiError, AiResult,
    service::semantic_search::{
        EmbeddingsReader, SearchHit, SemanticSearchService, TopHit as AiTopHit,
    },
};
use zk_domain::{AuthScope, Capability, ChunkId, EmbeddingModelId, EmbeddingVector};

use crate::{
    ServerDb, ServerError, auth::AuthenticatedDevice, http::ApiError, repo::EmbeddingsRepo,
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
    let service = SemanticSearchService::new(
        provider,
        ServerEmbeddingsReader {
            db: state.db_handle(),
        },
    );
    let hits = service
        .query(
            &request.query,
            request.top_k.unwrap_or(DEFAULT_TOP_K).min(MAX_TOP_K),
        )
        .await
        .map_err(ai_error_to_api_error)?;

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
        other => ApiError::from(ServerError::Internal(Box::new(io::Error::other(
            other.to_string(),
        )))),
    }
}

#[derive(Clone)]
struct ServerEmbeddingsReader {
    db: Arc<ServerDb>,
}

impl EmbeddingsReader for ServerEmbeddingsReader {
    fn top_k_cosine(
        &self,
        query: &EmbeddingVector,
        k: usize,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<AiTopHit>> {
        self.db
            .with_conn(|connection| {
                EmbeddingsRepo::new(connection)
                    .top_k_cosine(query, k, model)
                    .map(|hits| {
                        hits.into_iter()
                            .map(|hit| AiTopHit {
                                note_id: hit.note_id,
                                chunk_id: hit.chunk_id,
                                similarity: hit.similarity,
                            })
                            .collect()
                    })
            })
            .map_err(|error| AiError::Internal(format!("search server chunk embeddings: {error}")))
    }

    fn list_chunk_text(&self, chunk_ids: &[ChunkId]) -> AiResult<Vec<(ChunkId, String)>> {
        self.with_conn(|connection| {
            EmbeddingsReader::list_chunk_text(&EmbeddingsRepo::new(connection), chunk_ids)
        })
    }
}

impl ServerEmbeddingsReader {
    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> AiResult<R>) -> AiResult<R> {
        self.db
            .with_conn(|connection| f(connection).map_err(|error| ai_to_server_error(&error)))
            .map_err(|error| AiError::Internal(format!("read server chunk text: {error}")))
    }
}

fn ai_to_server_error(error: &AiError) -> ServerError {
    ServerError::Internal(Box::new(io::Error::other(error.to_string())))
}
