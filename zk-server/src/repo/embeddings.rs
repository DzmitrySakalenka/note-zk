use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
    io::{self, ErrorKind},
};

use rusqlite::{Connection, Row, types::Type};
use zk_domain::{
    ChunkEmbeddingRecord, ChunkId, EmbeddingModelId, EmbeddingVector, EmbeddingVectorError,
    ProviderId, decode_vector, encode_vector, id::NoteId,
};

use super::timestamps::{decode_timestamp, encode_timestamp};
use crate::{ServerError, ServerResult};

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

/// Repository over the server `chunk_embeddings` table.
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
    /// Returns [`ServerError`] if dimension conversion or the upsert fails.
    pub fn upsert(&self, record: &ChunkEmbeddingRecord) -> ServerResult<()> {
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
            .map_err(ServerError::Database)
    }

    /// Deletes all embeddings for chunks belonging to one note.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the delete fails.
    pub fn delete_for_note(&self, note_id: &NoteId) -> ServerResult<u64> {
        self.conn
            .execute(DELETE_EMBEDDINGS_FOR_NOTE_SQL, [note_id.as_str()])
            .map(rows_changed_to_u64)
            .map_err(ServerError::Database)
    }

    /// Lists all model embeddings stored for one chunk.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or a stored row is corrupt.
    pub fn list_for_chunk(&self, chunk_id: &ChunkId) -> ServerResult<Vec<ChunkEmbeddingRecord>> {
        let mut statement = self
            .conn
            .prepare_cached(LIST_EMBEDDINGS_FOR_CHUNK_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map([chunk_id.as_str()], decode_embedding_record)
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Returns the top `k` chunks by cosine similarity for a normalized query.
    ///
    /// This method assumes both stored vectors and the query vector are already
    /// L2-normalized, so the cosine score is the raw dot product. Passing an
    /// unnormalized query returns incorrect rankings.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or a stored vector is corrupt.
    pub fn top_k_cosine(
        &self,
        query: &EmbeddingVector,
        k: usize,
        model: &EmbeddingModelId,
    ) -> ServerResult<Vec<TopHit>> {
        if k == 0 {
            return Ok(Vec::new());
        }

        let dimension = dimension_to_i64(query.dim())?;
        let mut statement = self
            .conn
            .prepare_cached(TOP_K_COSINE_SQL)
            .map_err(ServerError::Database)?;
        let mut rows = statement
            .query(rusqlite::params![model.as_str(), dimension])
            .map_err(ServerError::Database)?;
        let mut heap = BinaryHeap::with_capacity(k);

        while let Some(row) = rows.next().map_err(ServerError::Database)? {
            let scored_hit = decode_scored_hit(row, query).map_err(ServerError::Database)?;

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
    f32::from_le_bytes(chunk.try_into().expect("chunk should contain one f32"))
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

fn dimension_to_i64(dimension: usize) -> ServerResult<i64> {
    i64::try_from(dimension).map_err(|_| {
        ServerError::InvalidRequest("embedding dimension exceeds SQLite integer range".to_string())
    })
}

fn rows_changed_to_u64(rows_changed: usize) -> u64 {
    u64::try_from(rows_changed).expect("row count should fit u64")
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
