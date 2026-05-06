use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
    io::{self, ErrorKind},
    mem::size_of,
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};
use zk_ai::{
    AiError, AiResult,
    service::{
        embeddings::EmbeddingsWriter,
        semantic_search::{EmbeddingsReader, TopHit as AiTopHit},
    },
};
use zk_domain::{
    ChunkEmbeddingRecord, ChunkId, ChunkRecord, EmbeddingModelId, EmbeddingVector,
    EmbeddingVectorError, ProviderId, decode_vector, encode_vector,
    id::{ContentHash, NoteId},
};

use crate::error::IndexError;

const DELETE_EMBEDDINGS_FOR_NOTE_SQL: &str = r"
DELETE FROM chunk_embeddings
WHERE chunk_id IN (
    SELECT chunk_id
    FROM note_chunks
    WHERE note_id = ?1
)
";
const INSERT_EMBEDDING_SQL: &str = r"
INSERT INTO chunk_embeddings (
    chunk_id,
    embedding_model,
    provider_id,
    dim,
    vector,
    created_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6
)
ON CONFLICT (chunk_id, embedding_model) DO UPDATE SET
    provider_id = excluded.provider_id,
    dim = excluded.dim,
    vector = excluded.vector,
    created_at = excluded.created_at
";
const LIST_EMBEDDINGS_FOR_CHUNK_SQL: &str = r"
SELECT
    chunk_id,
    embedding_model,
    provider_id,
    dim,
    vector,
    created_at
FROM chunk_embeddings
WHERE chunk_id = ?1
ORDER BY embedding_model ASC
";
const LIST_CHUNKS_MISSING_EMBEDDINGS_SQL: &str = r"
SELECT
    nc.chunk_id,
    nc.note_id,
    nc.ordinal,
    nc.content_hash,
    nc.text,
    nc.token_count,
    nc.created_at
FROM note_chunks AS nc
LEFT JOIN chunk_embeddings AS ce
    ON ce.chunk_id = nc.chunk_id AND ce.embedding_model = ?2
WHERE nc.note_id = ?1 AND ce.chunk_id IS NULL
ORDER BY nc.ordinal ASC
";
const GET_CHUNK_TEXT_SQL: &str = "SELECT text FROM note_chunks WHERE chunk_id = ?1";
const TOP_K_COSINE_SQL: &str = r"
SELECT
    nc.note_id,
    ce.chunk_id,
    ce.vector
FROM chunk_embeddings AS ce
INNER JOIN note_chunks AS nc ON nc.chunk_id = ce.chunk_id
WHERE ce.embedding_model = ?1 AND ce.dim = ?2
";
const FLOAT_BYTE_LEN: usize = size_of::<f32>();

/// Semantic search hit returned by brute-force cosine search.
#[derive(Debug, Clone, PartialEq)]
pub struct TopHit {
    /// Source note that owns the matching chunk.
    pub note_id: NoteId,
    /// Matching chunk identifier.
    pub chunk_id: ChunkId,
    /// Dot-product similarity between normalized vectors.
    pub similarity: f32,
}

/// Repository over the local `chunk_embeddings` table.
pub struct EmbeddingsRepo<'a> {
    conn: &'a Connection,
}

impl<'a> EmbeddingsRepo<'a> {
    /// Creates a repository backed by one `SQLite` connection.
    #[must_use]
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Inserts or replaces one chunk embedding.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if dimension conversion or the upsert fails.
    pub fn upsert(&self, record: &ChunkEmbeddingRecord) -> Result<(), IndexError> {
        let vector = encode_vector(&record.vector);
        let created_at = encode_timestamp(record.created_at);
        let dimension = dimension_to_i64(record.vector.dim())?;

        self.conn
            .execute(
                INSERT_EMBEDDING_SQL,
                rusqlite::params![
                    record.chunk_id.as_str(),
                    record.model.as_str(),
                    record.provider.as_str(),
                    dimension,
                    vector,
                    created_at,
                ],
            )
            .map(|_| ())
            .map_err(|source| sqlite_error(source, "upsert embedding"))
    }

    /// Deletes all embeddings for chunks belonging to one note.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the delete fails.
    pub fn delete_for_note(&self, note_id: &NoteId) -> Result<u64, IndexError> {
        self.conn
            .execute(DELETE_EMBEDDINGS_FOR_NOTE_SQL, [note_id.as_str()])
            .map(rows_changed_to_u64)
            .map_err(|source| sqlite_error(source, "delete embeddings for note"))
    }

    /// Lists all model embeddings stored for one chunk.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or a stored row is corrupt.
    pub fn list_for_chunk(
        &self,
        chunk_id: &ChunkId,
    ) -> Result<Vec<ChunkEmbeddingRecord>, IndexError> {
        let mut statement = self
            .conn
            .prepare_cached(LIST_EMBEDDINGS_FOR_CHUNK_SQL)
            .map_err(|source| sqlite_error(source, "prepare list embeddings for chunk"))?;
        let rows = statement
            .query_map([chunk_id.as_str()], decode_embedding_record)
            .map_err(|source| sqlite_error(source, "list embeddings for chunk"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|source| embedding_sqlite_error(source, "decode embeddings for chunk"))
    }

    /// Returns the top `k` chunks by cosine similarity for a normalized query.
    ///
    /// This method assumes both stored vectors and the query vector are already
    /// L2-normalized, so the cosine score is the raw dot product. Passing an
    /// unnormalized query returns incorrect rankings.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or a stored vector is corrupt.
    pub fn top_k_cosine(
        &self,
        query: &EmbeddingVector,
        k: usize,
        model: &EmbeddingModelId,
    ) -> Result<Vec<TopHit>, IndexError> {
        if k == 0 {
            return Ok(Vec::new());
        }

        let dimension = dimension_to_i64(query.dim())?;
        let mut statement = self
            .conn
            .prepare_cached(TOP_K_COSINE_SQL)
            .map_err(|source| sqlite_error(source, "prepare top-k cosine search"))?;
        let mut rows = statement
            .query(rusqlite::params![model.as_str(), dimension])
            .map_err(|source| sqlite_error(source, "query top-k cosine search"))?;
        let mut heap = BinaryHeap::with_capacity(k);

        while let Some(row) = rows
            .next()
            .map_err(|source| sqlite_error(source, "read top-k cosine row"))?
        {
            let scored_hit = decode_scored_hit(row, query)
                .map_err(|source| embedding_sqlite_error(source, "decode top-k cosine row"))?;

            if heap.len() < k {
                heap.push(Reverse(scored_hit));
            } else if let Some(mut lowest_hit) = heap.peek_mut()
                && scored_hit.cmp(&lowest_hit.0).is_gt()
            {
                *lowest_hit = Reverse(scored_hit);
            }
        }

        let mut scored_hits = heap
            .into_iter()
            .map(|Reverse(scored_hit)| scored_hit)
            .collect::<Vec<_>>();
        scored_hits.sort_by(|left, right| right.cmp(left));

        Ok(scored_hits
            .into_iter()
            .map(ScoredHit::into_top_hit)
            .collect())
    }
}

impl EmbeddingsReader for EmbeddingsRepo<'_> {
    fn top_k_cosine(
        &self,
        query: &EmbeddingVector,
        k: usize,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<AiTopHit>> {
        EmbeddingsRepo::top_k_cosine(self, query, k, model)
            .map(|hits| {
                hits.into_iter()
                    .map(|hit| AiTopHit {
                        note_id: hit.note_id,
                        chunk_id: hit.chunk_id,
                        similarity: hit.similarity,
                    })
                    .collect()
            })
            .map_err(local_search_error_to_ai_error)
    }

    fn list_chunk_text(&self, chunk_ids: &[ChunkId]) -> AiResult<Vec<(ChunkId, String)>> {
        if chunk_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut statement = self
            .conn
            .prepare_cached(GET_CHUNK_TEXT_SQL)
            .map_err(|error| AiError::Internal(format!("prepare get local chunk text: {error}")))?;
        let mut texts = Vec::with_capacity(chunk_ids.len());

        for chunk_id in chunk_ids {
            let Some(text) = statement
                .query_row([chunk_id.as_str()], |row| row.get::<_, String>(0))
                .optional()
                .map_err(|error| AiError::Internal(format!("get local chunk text: {error}")))?
            else {
                continue;
            };

            texts.push((chunk_id.clone(), text));
        }

        Ok(texts)
    }
}

impl EmbeddingsWriter for EmbeddingsRepo<'_> {
    fn list_chunks_missing_embeddings(
        &self,
        note_id: &NoteId,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<ChunkRecord>> {
        let mut statement = self
            .conn
            .prepare_cached(LIST_CHUNKS_MISSING_EMBEDDINGS_SQL)
            .map_err(|error| {
                AiError::Internal(format!(
                    "prepare list local chunks missing embeddings: {error}"
                ))
            })?;
        let rows = statement
            .query_map(
                rusqlite::params![note_id.as_str(), model.as_str()],
                decode_chunk_record,
            )
            .map_err(|error| {
                AiError::Internal(format!("list local chunks missing embeddings: {error}"))
            })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            AiError::Internal(format!("decode local chunks missing embeddings: {error}"))
        })
    }

    fn upsert_embeddings(&self, records: &[ChunkEmbeddingRecord]) -> AiResult<u64> {
        for record in records {
            self.upsert(record).map_err(|error| {
                AiError::Internal(format!("upsert local chunk embedding: {error}"))
            })?;
        }

        Ok(u64::try_from(records.len()).expect("record count should fit u64"))
    }
}

#[derive(Debug, Clone)]
struct ScoredHit {
    note_id: NoteId,
    chunk_id: ChunkId,
    similarity: f32,
}

impl ScoredHit {
    fn into_top_hit(self) -> TopHit {
        TopHit {
            note_id: self.note_id,
            chunk_id: self.chunk_id,
            similarity: self.similarity,
        }
    }
}

impl Eq for ScoredHit {}

impl PartialEq for ScoredHit {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Ord for ScoredHit {
    fn cmp(&self, other: &Self) -> Ordering {
        self.similarity
            .total_cmp(&other.similarity)
            .then_with(|| self.chunk_id.as_str().cmp(other.chunk_id.as_str()))
            .then_with(|| self.note_id.as_str().cmp(other.note_id.as_str()))
    }
}

impl PartialOrd for ScoredHit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn decode_embedding_record(row: &Row<'_>) -> rusqlite::Result<ChunkEmbeddingRecord> {
    let chunk_id = row.get::<_, String>(0)?;
    let model = decode_model(row, 1)?;
    let provider = decode_provider(row, 2)?;
    let stored_dimension = decode_usize(row, 3)?;
    let vector_bytes = row.get::<_, Vec<u8>>(4)?;
    let vector = decode_vector(&vector_bytes, model.clone())
        .map_err(|source| from_sql_conversion_failure(4, Type::Blob, source))?;

    ensure_stored_dimension_matches_vector(stored_dimension, vector.dim())?;

    Ok(ChunkEmbeddingRecord {
        chunk_id: ChunkId::new(&chunk_id),
        model,
        provider,
        vector,
        created_at: decode_timestamp(row, 5)?,
    })
}

fn decode_scored_hit(row: &Row<'_>, query: &EmbeddingVector) -> rusqlite::Result<ScoredHit> {
    let note_id = row.get::<_, String>(0)?;
    let chunk_id = row.get::<_, String>(1)?;
    let vector_bytes = row.get::<_, Vec<u8>>(2)?;
    let similarity = dot_product_with_encoded_vector(&vector_bytes, query)?;

    Ok(ScoredHit {
        note_id: NoteId::new(&note_id),
        chunk_id: ChunkId::new(&chunk_id),
        similarity,
    })
}

fn decode_chunk_record(row: &Row<'_>) -> rusqlite::Result<ChunkRecord> {
    let chunk_id = row.get::<_, String>(0)?;
    let note_id = row.get::<_, String>(1)?;
    let content_hash = row.get::<_, String>(3)?;

    Ok(ChunkRecord {
        chunk_id: ChunkId::new(&chunk_id),
        note_id: NoteId::new(&note_id),
        ordinal: decode_u32(row, 2)?,
        content_hash: ContentHash::new(&content_hash),
        text: row.get(4)?,
        token_count: decode_optional_u32(row, 5)?,
        created_at: decode_timestamp(row, 6)?,
    })
}

fn dot_product_with_encoded_vector(bytes: &[u8], query: &EmbeddingVector) -> rusqlite::Result<f32> {
    ensure_encoded_dimension_matches_query(bytes, query.dim())?;

    #[cfg(target_endian = "little")]
    if let Ok(values) = bytemuck::try_cast_slice::<u8, f32>(bytes) {
        return Ok(dot_product(values, &query.values));
    }

    let mut sum = 0.0;
    for (chunk, query_value) in bytes.chunks_exact(FLOAT_BYTE_LEN).zip(query.values.iter()) {
        let value = f32_from_le_chunk(chunk);
        sum += value * query_value;
    }

    Ok(sum)
}

fn f32_from_le_chunk(chunk: &[u8]) -> f32 {
    f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
}

fn dot_product(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .fold(0.0, |sum, (left_value, right_value)| {
            sum + (left_value * right_value)
        })
}

fn ensure_encoded_dimension_matches_query(
    bytes: &[u8],
    query_dimension: usize,
) -> rusqlite::Result<()> {
    if !is_valid_embedding_blob_len(bytes.len()) {
        return Err(from_sql_conversion_failure(
            2,
            Type::Blob,
            EmbeddingVectorError::InvalidBlob { len: bytes.len() },
        ));
    }

    let expected_len = query_dimension
        .checked_mul(FLOAT_BYTE_LEN)
        .ok_or_else(|| invalid_embedding_dimension_failure(query_dimension))?;

    if bytes.len() == expected_len {
        return Ok(());
    }

    Err(from_sql_conversion_failure(
        2,
        Type::Blob,
        EmbeddingVectorError::DimensionMismatch {
            expected: query_dimension,
            actual: bytes.len() / FLOAT_BYTE_LEN,
        },
    ))
}

fn ensure_stored_dimension_matches_vector(
    stored_dimension: usize,
    vector_dimension: usize,
) -> rusqlite::Result<()> {
    if stored_dimension == vector_dimension {
        return Ok(());
    }

    Err(from_sql_conversion_failure(
        3,
        Type::Integer,
        EmbeddingVectorError::DimensionMismatch {
            expected: stored_dimension,
            actual: vector_dimension,
        },
    ))
}

fn encode_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339()
}

fn decode_timestamp(row: &Row<'_>, index: usize) -> rusqlite::Result<DateTime<Utc>> {
    let raw_timestamp = row.get::<_, String>(index)?;

    DateTime::parse_from_rfc3339(&raw_timestamp)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|source| {
            from_sql_conversion_failure(
                index,
                Type::Text,
                io::Error::new(ErrorKind::InvalidData, source.to_string()),
            )
        })
}

fn decode_model(row: &Row<'_>, index: usize) -> rusqlite::Result<EmbeddingModelId> {
    let value = row.get::<_, String>(index)?;
    value
        .parse()
        .map_err(|source| from_sql_conversion_failure(index, Type::Text, source))
}

fn decode_provider(row: &Row<'_>, index: usize) -> rusqlite::Result<ProviderId> {
    let value = row.get::<_, String>(index)?;
    value
        .parse()
        .map_err(|source| from_sql_conversion_failure(index, Type::Text, source))
}

fn decode_usize(row: &Row<'_>, index: usize) -> rusqlite::Result<usize> {
    let value = row.get::<_, i64>(index)?;
    usize::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn decode_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value = row.get::<_, i64>(index)?;
    u32::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn decode_optional_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u32>> {
    let Some(value) = row.get::<_, Option<i64>>(index)? else {
        return Ok(None);
    };

    u32::try_from(value)
        .map(Some)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn dimension_to_i64(dimension: usize) -> Result<i64, IndexError> {
    i64::try_from(dimension).map_err(|_| {
        sqlite_error(
            rusqlite::Error::ToSqlConversionFailure(Box::new(io::Error::new(
                ErrorKind::InvalidInput,
                "embedding dimension exceeds SQLite integer range",
            ))),
            "convert embedding dimension",
        )
    })
}

fn rows_changed_to_u64(rows_changed: usize) -> u64 {
    u64::try_from(rows_changed).expect("row count should fit u64")
}

fn is_valid_embedding_blob_len(len: usize) -> bool {
    len > 0 && len % FLOAT_BYTE_LEN == 0
}

fn invalid_embedding_dimension_failure(dimension: usize) -> rusqlite::Error {
    from_sql_conversion_failure(
        2,
        Type::Integer,
        io::Error::new(
            ErrorKind::InvalidData,
            format!("embedding dimension {dimension} overflows byte length"),
        ),
    )
}

fn from_sql_conversion_failure(
    index: usize,
    source_type: Type,
    source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, source_type, source.into())
}

fn sqlite_error(source: rusqlite::Error, context: &str) -> IndexError {
    IndexError::Sqlite {
        source,
        context: context.to_string(),
    }
}

fn embedding_sqlite_error(source: rusqlite::Error, context: &str) -> IndexError {
    if let Some(len) = invalid_embedding_blob_len(&source) {
        return IndexError::InvalidEmbeddingBlob { len };
    }

    sqlite_error(source, context)
}

fn invalid_embedding_blob_len(source: &rusqlite::Error) -> Option<usize> {
    let rusqlite::Error::FromSqlConversionFailure(_, Type::Blob, source) = source else {
        return None;
    };
    let error = source.downcast_ref::<EmbeddingVectorError>()?;

    match error {
        EmbeddingVectorError::InvalidBlob { len } => Some(*len),
        EmbeddingVectorError::DimensionMismatch { .. } => None,
    }
}

fn local_search_error_to_ai_error(error: IndexError) -> AiError {
    match error {
        IndexError::InvalidEmbeddingBlob { len } => AiError::InvalidEmbeddingBlob { len },
        other => AiError::Internal(format!("search local chunk embeddings: {other}")),
    }
}
