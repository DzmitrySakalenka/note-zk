use std::{collections::BTreeMap, fmt::Debug};

use serde::{Serialize, de::DeserializeOwned};
use zk_domain::{
    id::{NoteId, RelativePath},
    note::NoteFrontmatter,
    write::{BatchWriteRequest, CreateNoteRequest, UpdateNoteRequest},
};

fn assert_common_traits<T>()
where
    T: Debug + Clone + PartialEq + Serialize + DeserializeOwned,
{
}

fn sample_frontmatter(id: &str, title: &str) -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::from(id),
        title: title.to_string(),
        tags: vec!["tg8".to_string()],
        note_type: Some("note".to_string()),
        aliases: vec!["Alias".to_string()],
        created: Some("2026-04-17T00:00:00Z".to_string()),
        updated: Some("2026-04-17T00:00:00Z".to_string()),
        source: Some("https://example.com".to_string()),
        status: Some("draft".to_string()),
        sources: vec![NoteId::from("01SOURCE000000000000000001")],
        extra: BTreeMap::default(),
    }
}

#[test]
fn write_request_types_implement_required_common_traits() {
    assert_common_traits::<CreateNoteRequest>();
    assert_common_traits::<UpdateNoteRequest>();
    assert_common_traits::<BatchWriteRequest>();
}

#[test]
fn create_note_request_serde_roundtrip() {
    let request = CreateNoteRequest {
        path: RelativePath::new("notes/new.md"),
        frontmatter: sample_frontmatter("01CREATE000000000000000001", "Create"),
        body_markdown: "# Create\n\nBody\n".to_string(),
    };

    let json = serde_json::to_string(&request).expect("serialize CreateNoteRequest");
    let decoded: CreateNoteRequest =
        serde_json::from_str(&json).expect("deserialize CreateNoteRequest");

    assert_eq!(decoded, request);
}

#[test]
fn update_note_request_serde_roundtrip() {
    let request = UpdateNoteRequest {
        note_id: NoteId::from("01UPDATE000000000000000001"),
        frontmatter: Some(sample_frontmatter("01UPDATE000000000000000001", "Updated")),
        body_markdown: Some("# Updated\n\nNew body\n".to_string()),
    };

    let json = serde_json::to_string(&request).expect("serialize UpdateNoteRequest");
    let decoded: UpdateNoteRequest =
        serde_json::from_str(&json).expect("deserialize UpdateNoteRequest");

    assert_eq!(decoded, request);
}

#[test]
fn batch_write_request_serde_roundtrip() {
    let request = BatchWriteRequest {
        creates: vec![CreateNoteRequest {
            path: RelativePath::new("notes/a.md"),
            frontmatter: sample_frontmatter("01BATCH000000000000000001", "A"),
            body_markdown: "A body".to_string(),
        }],
        updates: vec![UpdateNoteRequest {
            note_id: NoteId::from("01BATCH000000000000000002"),
            frontmatter: None,
            body_markdown: Some("Updated body".to_string()),
        }],
    };

    let json = serde_json::to_string(&request).expect("serialize BatchWriteRequest");
    let decoded: BatchWriteRequest =
        serde_json::from_str(&json).expect("deserialize BatchWriteRequest");

    assert_eq!(decoded, request);
}
