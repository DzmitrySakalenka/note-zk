use crossterm::event::KeyCode;
use zk_app::{ItemKind, RelativePath, ScanOutcome};
use zk_domain::{
    asset::AssetRecord,
    id::{AssetId, ContentHash, NoteId, ServerRevision},
    note::{ValidationReport, ValidationSummary},
    sync_state::{DeleteRecord, DeleteRecordTarget},
};
use zk_index::{AssetRepo, DeleteRecordRepo, NoteRepo, UpsertNoteParams};
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{JobKind, JobResult, JobSpec, ScanArgs, TrashOutcome},
    screens::trash::TrashScreen,
};
use zk_vault::{
    AssetCandidate, NormalizationPlan, NormalizationResult, TitleSource, TrashState,
    VaultScanResult,
};

use super::helpers::{
    assert_screen_snapshot, background, job_error_event, key, render_screen, test_context,
};

const NOTE_ID: &str = "01HTRASHEDNOTE000000000001";
const ASSET_ID: &str = "01HTRASHEDASSET0000000001";
const TRASHED_AT: &str = "2026-05-01T12:00:00Z";
const EXPIRED_AT: &str = "2026-05-02T12:00:00Z";
const FUTURE_AT: &str = "2999-05-02T12:00:00Z";

#[test]
fn trash_empty_yaml_snapshot() {
    let mut ctx = trash_context();
    let mut screen = TrashScreen::default();
    assert_scan_action(&screen.on_enter(&mut ctx));
    assert!(matches!(
        screen.handle_event(&scan_event(empty_scan()), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__trash__empty", &render_screen(&mut screen, &ctx));
}

#[test]
fn trash_loaded_yaml_snapshot() {
    let mut ctx = trash_context();
    seed_mixed_trash(&mut ctx);
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(&scan_event(mixed_scan()), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__trash__loaded", &render_screen(&mut screen, &ctx));
}

#[test]
fn trash_dialog_yaml_snapshot() {
    let mut ctx = trash_context();
    seed_mixed_trash(&mut ctx);
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(&scan_event(mixed_scan()), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('D')), &mut ctx),
        ScreenAction::Stay
    ));

    assert_screen_snapshot(
        "screens__trash__purge_dialog",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn trash_loading_after_action_yaml_snapshot() {
    let mut ctx = trash_context();
    seed_mixed_trash(&mut ctx);
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(&scan_event(mixed_scan()), &mut ctx),
        ScreenAction::Stay
    ));
    assert_scan_action(&screen.handle_event(&restored_event(), &mut ctx));

    assert_screen_snapshot("screens__trash__loading", &render_screen(&mut screen, &ctx));
}

#[test]
fn trash_error_yaml_snapshot() {
    let mut ctx = trash_context();
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(
            &job_error_event(JobKind::ScanVault, "scan failed"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__trash__error", &render_screen(&mut screen, &ctx));
}

fn trash_context() -> zk_tui::app::ScreenCtx {
    let mut ctx = test_context();
    ctx.config.trash_dir = ".trash".to_string();
    ctx
}

fn seed_mixed_trash(ctx: &mut zk_tui::app::ScreenCtx) {
    let conn = ctx
        .connection
        .lock()
        .expect("test connection should lock for seeding");
    let note_id = NoteId::new(NOTE_ID);
    let note_path = RelativePath::new(".trash/expired.md");
    let asset_id = AssetId::new(ASSET_ID);
    let asset_path = RelativePath::new(".trash/diagram.png");
    let content_hash = ContentHash::from_bytes(b"trash fixture");

    NoteRepo::upsert_note(
        &conn,
        UpsertNoteParams {
            note_id: &note_id,
            path: &note_path,
            title: "Expired note",
            content_hash: &content_hash,
            frontmatter_json: "{}",
            created_at: "2026-04-01T00:00:00Z",
            updated_at: "2026-05-01T12:00:00Z",
            trashed_at: Some(TRASHED_AT),
            is_trashed: true,
        },
    )
    .expect("trashed note should seed");
    AssetRepo::upsert_asset(
        &conn,
        &AssetRecord {
            asset_id: asset_id.clone(),
            current_path: asset_path.clone(),
            content_hash: content_hash.clone(),
            mime_type: Some("image/png".to_string()),
            size_bytes: 128,
            created_at: "2026-04-01T00:00:00Z".to_string(),
            updated_at: "2026-05-01T12:00:00Z".to_string(),
            trashed_at: Some(TRASHED_AT.to_string()),
            deleted_at: None,
            is_trashed: true,
            is_deleted: false,
            server_revision: ServerRevision::new(0),
        },
    )
    .expect("trashed asset should seed");
    DeleteRecordRepo::insert_delete_record(
        &conn,
        &DeleteRecord {
            target: DeleteRecordTarget::Note(note_id),
            original_path: RelativePath::new("notes/expired.md"),
            trashed_at: TRASHED_AT.to_string(),
            retention_until: EXPIRED_AT.to_string(),
            deleted_at: None,
            delete_revision: None,
        },
    )
    .expect("note delete record should seed");
    DeleteRecordRepo::insert_delete_record(
        &conn,
        &DeleteRecord {
            target: DeleteRecordTarget::Asset(asset_id),
            original_path: RelativePath::new("assets/diagram.png"),
            trashed_at: TRASHED_AT.to_string(),
            retention_until: FUTURE_AT.to_string(),
            deleted_at: None,
            delete_revision: None,
        },
    )
    .expect("asset delete record should seed");
}

fn empty_scan() -> ScanOutcome {
    scan_outcome(Vec::new(), Vec::new())
}

fn mixed_scan() -> ScanOutcome {
    scan_outcome(
        vec![NormalizationResult {
            relative_path: RelativePath::new(".trash/expired.md"),
            note_id: Some(NoteId::new(NOTE_ID)),
            trash_state: TrashState::Trashed,
            trashed_at: Some(TRASHED_AT.to_string()),
            plan: clean_plan(),
            frontmatter: None,
            document: None,
            content_hash: ContentHash::from_bytes(b"note bytes"),
        }],
        vec![AssetCandidate {
            relative_path: RelativePath::new(".trash/diagram.png"),
            absolute_path: "/vault/.trash/diagram.png".into(),
            trash_state: TrashState::Trashed,
            trashed_at: Some(TRASHED_AT.to_string()),
        }],
    )
}

fn scan_outcome(notes: Vec<NormalizationResult>, assets: Vec<AssetCandidate>) -> ScanOutcome {
    ScanOutcome {
        scan_result: VaultScanResult {
            notes,
            assets,
            diagnostics: Vec::new(),
            frontmatter_parse_failures: Vec::new(),
            duplicate_ids: Vec::new(),
            trashed_note_count: 0,
        },
        parsed_indexes: Vec::new(),
        validation_report: ValidationReport {
            findings: Vec::new(),
            summary: ValidationSummary {
                error_count: 0,
                warning_count: 0,
                info_count: 0,
                rules_triggered: Vec::new(),
            },
            scan_diagnostics: Vec::new(),
        },
    }
}

fn clean_plan() -> NormalizationPlan {
    NormalizationPlan {
        needs_id_injection: false,
        needs_title_derivation: false,
        title_source: TitleSource::Frontmatter,
        needs_frontmatter_insertion: false,
        needs_field_reorder: false,
        is_immutable: false,
        diagnostics: Vec::new(),
    }
}

fn restored_event() -> zk_tui::event::AppEvent {
    background(JobResult::TrashOutcome(TrashOutcome::Restored(
        zk_app::RestoredItem {
            item_id: NOTE_ID.to_string(),
            kind: ItemKind::Note,
            restored_to: RelativePath::new("notes/expired.md"),
        },
    )))
}

fn scan_event(outcome: ScanOutcome) -> zk_tui::event::AppEvent {
    background(JobResult::ScanOutcome(outcome))
}

fn assert_scan_action(action: &ScreenAction) {
    let ScreenAction::EnqueueJob(JobSpec::ScanVault(ScanArgs {
        include_trashed: true,
    })) = action
    else {
        panic!("expected scan vault job");
    };
}
