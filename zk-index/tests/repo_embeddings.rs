// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_ai::service::{embeddings::EmbeddingsWriter, semantic_search::EmbeddingsReader};
use zk_domain::{
    ChunkEmbeddingRecord, ChunkId, ChunkRecord, EmbeddingModelId, EmbeddingVector, ProviderId,
    id::{ContentHash, NoteId, RelativePath},
};
use zk_index::{
    ChunksRepo, EmbeddingsRepo, IndexError, NoteRepo, UpsertNoteParams, ensure_schema, open_memory,
};

fn setup_connection() -> Connection {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema migration should succeed");
    connection
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn insert_note(connection: &Connection, note_id: &NoteId) {
    let path = RelativePath::new(&format!("notes/{note_id}.md"));
    let content_hash = ContentHash::from_bytes(note_id.as_str().as_bytes());
    let title = format!("Title {note_id}");
    let frontmatter_json = format!(r#"{{"id":"{note_id}","title":"Title {note_id}"}}"#);

    NoteRepo::upsert_note(
        connection,
        UpsertNoteParams {
            note_id,
            path: &path,
            title: &title,
            content_hash: &content_hash,
            frontmatter_json: &frontmatter_json,
            created_at: "2026-05-05T10:00:00Z",
            updated_at: "2026-05-05T10:05:00Z",
            trashed_at: None,
            is_trashed: false,
        },
    )
    .expect("note row should be inserted");
}

fn note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S")
}

fn other_note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6T")
}

fn chunk_fixture(note_id: &NoteId, chunk_id: &str, ordinal: u32, text: &str) -> ChunkRecord {
    ChunkRecord {
        chunk_id: ChunkId::new(chunk_id),
        note_id: note_id.clone(),
        ordinal,
        content_hash: ContentHash::from_bytes(text.as_bytes()),
        text: text.to_string(),
        token_count: Some(ordinal + 3),
        created_at: parse_timestamp("2026-05-05T10:10:00Z"),
    }
}

fn model(value: &str) -> EmbeddingModelId {
    value
        .parse()
        .expect("embedding model fixture should be valid")
}

fn provider(value: &str) -> ProviderId {
    value.parse().expect("provider fixture should be valid")
}

fn embedding_vector(model: &EmbeddingModelId, values: &[f32]) -> EmbeddingVector {
    EmbeddingVector {
        values: values.to_vec(),
        model: model.clone(),
    }
}

fn normalized_embedding_vector(model: &EmbeddingModelId, values: &[f32]) -> EmbeddingVector {
    let mut vector = embedding_vector(model, values);
    vector.normalize_in_place();
    vector
}

fn embedding_record(
    chunk_id: &ChunkId,
    model: &EmbeddingModelId,
    provider: &ProviderId,
    values: &[f32],
    created_at: &str,
) -> ChunkEmbeddingRecord {
    ChunkEmbeddingRecord {
        chunk_id: chunk_id.clone(),
        model: model.clone(),
        provider: provider.clone(),
        vector: embedding_vector(model, values),
        created_at: parse_timestamp(created_at),
    }
}

fn insert_chunks(connection: &Connection, note_id: &NoteId, chunks: &[ChunkRecord]) {
    ChunksRepo::new(connection)
        .replace_for_note(note_id, chunks)
        .expect("insert chunks should succeed");
}

fn seed_single_chunk(connection: &Connection) -> ChunkRecord {
    let note_id = note_id();
    insert_note(connection, &note_id);
    let chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "Chunk");
    insert_chunks(connection, &note_id, std::slice::from_ref(&chunk));
    chunk
}

fn insert_raw_embedding_blob(
    connection: &Connection,
    chunk_id: &ChunkId,
    model: &EmbeddingModelId,
    dimension: i64,
    bytes: &[u8],
) {
    let provider = provider("test-provider");

    connection
        .execute(
            r"
            INSERT INTO chunk_embeddings (
                chunk_id,
                embedding_model,
                provider_id,
                dim,
                vector,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ",
            rusqlite::params![
                chunk_id.as_str(),
                model.as_str(),
                provider.as_str(),
                dimension,
                bytes,
                "2026-05-05T11:00:00Z",
            ],
        )
        .expect("raw embedding fixture should insert");
}

fn malformed_embedding_blobs() -> [Vec<u8>; 3] {
    [Vec::new(), vec![1; 5], vec![2; 7]]
}

fn assert_invalid_embedding_blob(error: &IndexError, expected_len: usize) {
    assert!(
        matches!(error, IndexError::InvalidEmbeddingBlob { len } if *len == expected_len),
        "expected invalid embedding blob length {expected_len}, got {error:?}"
    );
}

#[test]
fn upsert_and_list_round_trip_preserves_vector_bits() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "Chunk");
    insert_chunks(&connection, &note_id, std::slice::from_ref(&chunk));
    let model = model("test/model-a");
    let provider = provider("test-provider");
    let record = embedding_record(
        &chunk.chunk_id,
        &model,
        &provider,
        &[1.0, -0.0, f32::from_bits(0x7fc0_0001), 2.5],
        "2026-05-05T11:00:00Z",
    );

    let repo = EmbeddingsRepo::new(&connection);
    repo.upsert(&record).expect("upsert should succeed");
    let loaded = repo
        .list_for_chunk(&chunk.chunk_id)
        .expect("list embeddings should succeed");

    assert_eq!(loaded, vec![record]);
}

#[test]
fn upsert_twice_replaces_existing_row() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "Chunk");
    insert_chunks(&connection, &note_id, std::slice::from_ref(&chunk));
    let model = model("test/model-a");
    let first_provider = provider("first-provider");
    let second_provider = provider("second-provider");
    let first = embedding_record(
        &chunk.chunk_id,
        &model,
        &first_provider,
        &[1.0, 0.0],
        "2026-05-05T11:00:00Z",
    );
    let second = embedding_record(
        &chunk.chunk_id,
        &model,
        &second_provider,
        &[0.25, 0.5, 0.75],
        "2026-05-05T12:00:00Z",
    );

    let repo = EmbeddingsRepo::new(&connection);
    repo.upsert(&first).expect("first upsert should succeed");
    repo.upsert(&second).expect("second upsert should succeed");
    let loaded = repo
        .list_for_chunk(&chunk.chunk_id)
        .expect("list embeddings should succeed");

    assert_eq!(loaded, vec![second]);
}

#[test]
fn delete_for_note_removes_only_matching_note_embeddings() {
    let connection = setup_connection();
    let note_id = note_id();
    let other_note_id = other_note_id();
    insert_note(&connection, &note_id);
    insert_note(&connection, &other_note_id);
    let chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "Chunk");
    let other_chunk = chunk_fixture(
        &other_note_id,
        "01JAR5A3PJEZ3SKG67PN8N6S6Q",
        0,
        "Other chunk",
    );
    insert_chunks(&connection, &note_id, std::slice::from_ref(&chunk));
    insert_chunks(
        &connection,
        &other_note_id,
        std::slice::from_ref(&other_chunk),
    );
    let model = model("test/model-a");
    let provider = provider("test-provider");
    let record = embedding_record(
        &chunk.chunk_id,
        &model,
        &provider,
        &[1.0, 0.0],
        "2026-05-05T11:00:00Z",
    );
    let other_record = embedding_record(
        &other_chunk.chunk_id,
        &model,
        &provider,
        &[0.0, 1.0],
        "2026-05-05T11:00:00Z",
    );

    let repo = EmbeddingsRepo::new(&connection);
    repo.upsert(&record).expect("upsert should succeed");
    repo.upsert(&other_record)
        .expect("other upsert should succeed");
    let deleted = repo
        .delete_for_note(&note_id)
        .expect("delete embeddings should succeed");
    let remaining = repo
        .list_for_chunk(&other_chunk.chunk_id)
        .expect("list other embeddings should succeed");

    assert_eq!(deleted, 1);
    assert!(
        repo.list_for_chunk(&chunk.chunk_id)
            .expect("list deleted embeddings should succeed")
            .is_empty()
    );
    assert_eq!(remaining, vec![other_record]);
}

#[test]
fn embeddings_writer_lists_chunks_missing_requested_model() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let first_chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "First");
    let second_chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Second");
    insert_chunks(
        &connection,
        &note_id,
        &[first_chunk.clone(), second_chunk.clone()],
    );
    let queried_model = model("test/model-a");
    let other_model = model("test/model-b");
    let provider = provider("test-provider");
    let repo = EmbeddingsRepo::new(&connection);
    repo.upsert(&embedding_record(
        &first_chunk.chunk_id,
        &queried_model,
        &provider,
        &[1.0, 0.0],
        "2026-05-05T11:00:00Z",
    ))
    .expect("first chunk upsert should succeed");
    repo.upsert(&embedding_record(
        &second_chunk.chunk_id,
        &other_model,
        &provider,
        &[0.0, 1.0],
        "2026-05-05T11:00:00Z",
    ))
    .expect("other model upsert should succeed");

    let missing = EmbeddingsWriter::list_chunks_missing_embeddings(&repo, &note_id, &queried_model)
        .expect("missing chunks should list");

    assert_eq!(missing, vec![second_chunk]);
}

#[test]
fn embeddings_writer_upserts_multiple_records() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let first_chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "First");
    let second_chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Second");
    insert_chunks(
        &connection,
        &note_id,
        &[first_chunk.clone(), second_chunk.clone()],
    );
    let model = model("test/model-a");
    let provider = provider("test-provider");
    let records = vec![
        embedding_record(
            &first_chunk.chunk_id,
            &model,
            &provider,
            &[1.0, 0.0],
            "2026-05-05T11:00:00Z",
        ),
        embedding_record(
            &second_chunk.chunk_id,
            &model,
            &provider,
            &[0.0, 1.0],
            "2026-05-05T11:00:00Z",
        ),
    ];

    let repo = EmbeddingsRepo::new(&connection);
    let upserted = EmbeddingsWriter::upsert_embeddings(&repo, &records)
        .expect("batch embedding upsert should succeed");

    assert_eq!(upserted, 2);
    assert_eq!(
        repo.list_for_chunk(&first_chunk.chunk_id)
            .expect("first chunk embeddings should list"),
        vec![records[0].clone()]
    );
    assert_eq!(
        repo.list_for_chunk(&second_chunk.chunk_id)
            .expect("second chunk embeddings should list"),
        vec![records[1].clone()]
    );
}

#[test]
fn embeddings_reader_top_k_cosine_maps_repo_hits() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "Chunk");
    insert_chunks(&connection, &note_id, std::slice::from_ref(&chunk));
    let model = model("test/model-a");
    let provider = provider("test-provider");
    let repo = EmbeddingsRepo::new(&connection);
    repo.upsert(&ChunkEmbeddingRecord {
        chunk_id: chunk.chunk_id.clone(),
        model: model.clone(),
        provider,
        vector: normalized_embedding_vector(&model, &[1.0, 0.0]),
        created_at: parse_timestamp("2026-05-05T11:00:00Z"),
    })
    .expect("embedding upsert should succeed");
    let query = normalized_embedding_vector(&model, &[1.0, 0.0]);

    let hits = EmbeddingsReader::top_k_cosine(&repo, &query, 1, &model)
        .expect("reader top-k should succeed");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].note_id, note_id);
    assert_eq!(hits[0].chunk_id, chunk.chunk_id);
    assert!((hits[0].similarity - 1.0).abs() <= 0.000_001);
}

#[test]
fn embeddings_reader_lists_chunk_text_in_requested_order() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let first_chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "First");
    let second_chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Second");
    insert_chunks(
        &connection,
        &note_id,
        &[first_chunk.clone(), second_chunk.clone()],
    );
    let requested = vec![second_chunk.chunk_id.clone(), first_chunk.chunk_id.clone()];

    let texts = EmbeddingsReader::list_chunk_text(&EmbeddingsRepo::new(&connection), &requested)
        .expect("chunk text should list");

    assert_eq!(
        texts,
        vec![
            (second_chunk.chunk_id, "Second".to_string()),
            (first_chunk.chunk_id, "First".to_string()),
        ]
    );
}

#[test]
fn list_for_chunk_rejects_malformed_embedding_blobs() {
    for bytes in malformed_embedding_blobs() {
        let connection = setup_connection();
        let chunk = seed_single_chunk(&connection);
        let model = model("test/model-a");
        insert_raw_embedding_blob(&connection, &chunk.chunk_id, &model, 1, &bytes);

        let error = EmbeddingsRepo::new(&connection)
            .list_for_chunk(&chunk.chunk_id)
            .expect_err("malformed embedding blob should fail list decoding");

        assert_invalid_embedding_blob(&error, bytes.len());
    }
}

#[test]
fn top_k_cosine_rejects_malformed_embedding_blobs() {
    for bytes in malformed_embedding_blobs() {
        let connection = setup_connection();
        let chunk = seed_single_chunk(&connection);
        let model = model("test/model-a");
        insert_raw_embedding_blob(&connection, &chunk.chunk_id, &model, 1, &bytes);
        let query = normalized_embedding_vector(&model, &[1.0]);

        let error = EmbeddingsRepo::new(&connection)
            .top_k_cosine(&query, 1, &model)
            .expect_err("malformed embedding blob should fail top-k search");

        assert_invalid_embedding_blob(&error, bytes.len());
    }
}

#[test]
fn top_k_cosine_returns_expected_ordering() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let model = model("test/model-a");
    let provider = provider("test-provider");
    let chunks = (0_u32..10)
        .map(|ordinal| {
            chunk_fixture(
                &note_id,
                &format!("01JAR5A3PJEZ3SKG67PN8N6S6{ordinal}"),
                ordinal,
                &format!("Chunk {ordinal}"),
            )
        })
        .collect::<Vec<_>>();
    insert_chunks(&connection, &note_id, &chunks);
    let raw_vectors = [
        [1.0, 0.0],
        [0.8, 0.2],
        [0.6, 0.4],
        [0.4, 0.6],
        [0.2, 0.8],
        [0.0, 1.0],
        [-0.2, 0.8],
        [-0.4, 0.6],
        [-0.6, 0.4],
        [-0.8, 0.2],
    ];
    let records = chunks
        .iter()
        .zip(raw_vectors.iter())
        .map(|(chunk, values)| ChunkEmbeddingRecord {
            chunk_id: chunk.chunk_id.clone(),
            model: model.clone(),
            provider: provider.clone(),
            vector: normalized_embedding_vector(&model, values),
            created_at: parse_timestamp("2026-05-05T11:00:00Z"),
        })
        .collect::<Vec<_>>();
    let repo = EmbeddingsRepo::new(&connection);
    for record in &records {
        repo.upsert(record).expect("upsert should succeed");
    }
    let query = normalized_embedding_vector(&model, &[0.75, 0.25]);

    let hits = repo
        .top_k_cosine(&query, 4, &model)
        .expect("top-k search should succeed");
    let expected: Vec<(ChunkId, f32)> = {
        let mut scored = records
            .iter()
            .map(|record| {
                (
                    record.chunk_id.clone(),
                    record
                        .vector
                        .cosine_similarity(&query)
                        .expect("fixture dimensions should match"),
                )
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| right.1.total_cmp(&left.1));
        scored.into_iter().take(4).collect::<Vec<_>>()
    };

    assert_eq!(hits.len(), expected.len());
    for (hit, (expected_chunk_id, expected_similarity)) in hits.iter().zip(expected.iter()) {
        assert_eq!(&hit.note_id, &note_id);
        assert_eq!(&hit.chunk_id, expected_chunk_id);
        assert!((hit.similarity - expected_similarity).abs() <= 0.000_001);
    }
}

#[test]
fn top_k_cosine_empty_table_returns_empty_vec() {
    let connection = setup_connection();
    let model = model("test/model-a");
    let query = normalized_embedding_vector(&model, &[1.0, 0.0]);

    let hits = EmbeddingsRepo::new(&connection)
        .top_k_cosine(&query, 5, &model)
        .expect("top-k search should succeed");

    assert!(hits.is_empty());
}

#[test]
fn top_k_cosine_filters_by_model() {
    let connection = setup_connection();
    let note_id = note_id();
    insert_note(&connection, &note_id);
    let first_chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "First");
    let second_chunk = chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Second");
    insert_chunks(
        &connection,
        &note_id,
        &[first_chunk.clone(), second_chunk.clone()],
    );
    let queried_model = model("test/model-a");
    let other_model = model("test/model-b");
    let provider = provider("test-provider");
    let queried_record = ChunkEmbeddingRecord {
        chunk_id: first_chunk.chunk_id.clone(),
        model: queried_model.clone(),
        provider: provider.clone(),
        vector: normalized_embedding_vector(&queried_model, &[1.0, 0.0]),
        created_at: parse_timestamp("2026-05-05T11:00:00Z"),
    };
    let other_record = ChunkEmbeddingRecord {
        chunk_id: second_chunk.chunk_id.clone(),
        model: other_model.clone(),
        provider,
        vector: normalized_embedding_vector(&other_model, &[1.0, 0.0]),
        created_at: parse_timestamp("2026-05-05T11:00:00Z"),
    };
    let repo = EmbeddingsRepo::new(&connection);
    repo.upsert(&queried_record)
        .expect("queried model upsert should succeed");
    repo.upsert(&other_record)
        .expect("other model upsert should succeed");
    let query = normalized_embedding_vector(&queried_model, &[1.0, 0.0]);

    let hits = repo
        .top_k_cosine(&query, 5, &queried_model)
        .expect("top-k search should succeed");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk_id, first_chunk.chunk_id);
    assert_eq!(hits[0].note_id, note_id);
}
