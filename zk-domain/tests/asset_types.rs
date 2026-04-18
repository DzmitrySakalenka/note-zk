use zk_domain::asset::AssetRecord;
use zk_domain::id::{AssetId, ContentHash, RelativePath, ServerRevision};

// ==========================================================================
// AssetRecord -- construction and field access
// ==========================================================================

fn sample_asset() -> AssetRecord {
    AssetRecord {
        asset_id: AssetId::new("01HZ3ASSET00000000000000A"),
        current_path: RelativePath::new("assets/diagram.png"),
        content_hash: ContentHash::from_bytes(b"fake png bytes"),
        mime_type: Some("image/png".to_string()),
        size_bytes: 204_800,
        created_at: "2026-04-02T10:00:00Z".to_string(),
        updated_at: "2026-04-02T12:00:00Z".to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        server_revision: ServerRevision::new(1),
    }
}

#[test]
fn asset_record_construction() {
    let rec = sample_asset();
    assert_eq!(rec.asset_id.to_string(), "01HZ3ASSET00000000000000A");
    assert_eq!(rec.current_path.as_str(), "assets/diagram.png");
    assert_eq!(rec.mime_type.as_deref(), Some("image/png"));
    assert_eq!(rec.size_bytes, 204_800);
    assert!(!rec.is_trashed);
    assert!(!rec.is_deleted);
    assert!(rec.trashed_at.is_none());
    assert!(rec.deleted_at.is_none());
    assert_eq!(rec.server_revision.value(), 1);
}

#[test]
fn asset_record_without_mime_type() {
    let rec = AssetRecord {
        mime_type: None,
        ..sample_asset()
    };
    assert!(rec.mime_type.is_none());
}

#[test]
fn asset_record_with_trash_state() {
    let rec = AssetRecord {
        trashed_at: Some("2026-04-05T08:00:00Z".to_string()),
        is_trashed: true,
        ..sample_asset()
    };
    assert!(rec.is_trashed);
    assert!(!rec.is_deleted);
    assert_eq!(rec.trashed_at.as_deref(), Some("2026-04-05T08:00:00Z"));
    assert!(rec.deleted_at.is_none());
}

#[test]
fn asset_record_with_deleted_state() {
    let rec = AssetRecord {
        trashed_at: Some("2026-04-05T08:00:00Z".to_string()),
        deleted_at: Some("2026-04-12T08:00:00Z".to_string()),
        is_trashed: true,
        is_deleted: true,
        ..sample_asset()
    };
    assert!(rec.is_trashed);
    assert!(rec.is_deleted);
    assert_eq!(rec.deleted_at.as_deref(), Some("2026-04-12T08:00:00Z"));
}

#[test]
fn asset_record_clone_and_eq() {
    let a = sample_asset();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn asset_record_different_content_hash_not_equal() {
    let a = sample_asset();
    let b = AssetRecord {
        content_hash: ContentHash::from_bytes(b"different bytes"),
        ..sample_asset()
    };
    assert_ne!(a, b);
}

#[test]
fn asset_record_size_bytes_is_u64() {
    let rec = AssetRecord {
        size_bytes: u64::MAX,
        ..sample_asset()
    };
    assert_eq!(rec.size_bytes, u64::MAX);
}

#[test]
fn asset_record_has_no_body_or_frontmatter() {
    // Assets are binary files -- identity lives in the index only (ADR 0011).
    // This test is structural: if someone adds content/frontmatter fields,
    // sample_asset() will fail to compile.
    let rec = sample_asset();
    assert_eq!(rec.asset_id.to_string(), "01HZ3ASSET00000000000000A");
}

#[test]
fn asset_record_path_normalization() {
    let rec = AssetRecord {
        current_path: RelativePath::new(r"assets\images\photo.jpg"),
        ..sample_asset()
    };
    assert_eq!(rec.current_path.as_str(), "assets/images/photo.jpg");
}
