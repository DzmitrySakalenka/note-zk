#![allow(unused_crate_dependencies)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Frame, Terminal, backend::TestBackend, layout::Rect, widgets::Paragraph};
use zk_app::{
    ItemKind, PurgeRequest, PurgeScope, RelativePath, RestoreRequest, RestoredItem, ScanOutcome,
    TargetSelector,
};
use zk_domain::{
    asset::AssetRecord,
    id::{AssetId, ContentHash, NoteId, ServerRevision},
    note::{ValidationReport, ValidationSummary},
    sync_state::{DeleteRecord, DeleteRecordTarget},
};
use zk_index::{AssetRepo, DeleteRecordRepo, NoteRepo, UpsertNoteParams};
use zk_tui::{
    app::{App, Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{JobResult, JobSpec, ScanArgs, TrashIntent, TrashOutcome},
    screens::trash::TrashScreen,
};
use zk_vault::{
    AssetCandidate, NormalizationPlan, NormalizationResult, TitleSource, TrashState,
    VaultScanResult,
};

const WIDTH: u16 = 116;
const HEIGHT: u16 = 22;
const NOTE_ID: &str = "01HTRASHEDNOTE000000000001";
const ASSET_ID: &str = "01HTRASHEDASSET0000000001";
const TRASHED_AT: &str = "2026-05-01T12:00:00Z";
const EXPIRED_AT: &str = "2026-05-02T12:00:00Z";
const FUTURE_AT: &str = "2999-05-02T12:00:00Z";

#[test]
fn trash_enqueues_trashed_scan_on_entry() {
    let mut ctx = test_context();
    let mut screen = TrashScreen::default();

    assert_scan_action(&screen.on_enter(&mut ctx));
}

#[test]
fn trash_empty_snapshot() {
    let mut ctx = test_context();
    let mut screen = TrashScreen::default();
    assert_scan_action(&screen.on_enter(&mut ctx));
    assert!(matches!(
        screen.handle_event(&scan_event(empty_scan()), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Actions───────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│[j/k] Move  [r] Restore  [R] Restore to path  [d] Purge  [D] Purge expired  [t] Filter: all  [a] Kind: note       │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Trash─────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│Trash is empty.                                                                                                   │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn trash_mixed_list_snapshot() {
    let mut ctx = test_context();
    seed_mixed_trash(&mut ctx);
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(&scan_event(mixed_scan()), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Actions───────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│[j/k] Move  [r] Restore  [R] Restore to path  [d] Purge  [D] Purge expired  [t] Filter: all  [a] Kind: note       │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Trash─────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│Kind   Title or name              Original path              Trashed at            Expires at            Expired  │
│> note  Expired note               notes/expired.md           2026-05-01T12:00:00Z  2026-05-02T12:00:00Z  yes     │
│  asset diagram.png                assets/diagram.png         2026-05-01T12:00:00Z  2999-05-02T12:00:00Z  no      │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn trash_expired_filter_snapshot() {
    let mut ctx = test_context();
    seed_mixed_trash(&mut ctx);
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(&scan_event(mixed_scan()), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('t')), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Actions───────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│[j/k] Move  [r] Restore  [R] Restore to path  [d] Purge  [D] Purge expired  [t] Filter: expired  [a] Kind: note   │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Trash─────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│Kind   Title or name              Original path              Trashed at            Expires at            Expired  │
│> note  Expired note               notes/expired.md           2026-05-01T12:00:00Z  2026-05-02T12:00:00Z  yes     │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn trash_restore_refreshes_after_success() {
    let mut ctx = test_context();
    seed_mixed_trash(&mut ctx);
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(&scan_event(mixed_scan()), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('r')), &mut ctx),
        ScreenAction::Stay
    ));
    assert_restore_note_action(
        &screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(NOTE_ID.to_string()),
            alternate_path: None,
        },
    );
    assert_scan_action(&screen.handle_event(&restored_event(), &mut ctx));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Actions───────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│[j/k] Move  [r] Restore  [R] Restore to path  [d] Purge  [D] Purge expired  [t] Filter: all  [a] Kind: note       │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Trash─────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│loading trash...                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn trash_alternate_restore_input_sets_path() {
    let mut ctx = test_context();
    seed_mixed_trash(&mut ctx);
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(&scan_event(mixed_scan()), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('R')), &mut ctx),
        ScreenAction::Stay
    ));
    for character in "notes/restored.md".chars() {
        assert!(matches!(
            screen.handle_event(&key(KeyCode::Char(character)), &mut ctx),
            ScreenAction::Stay
        ));
    }

    assert_restore_note_action(
        &screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(NOTE_ID.to_string()),
            alternate_path: Some(RelativePath::new("notes/restored.md")),
        },
    );
}

#[test]
fn trash_purge_selected_confirm_enqueues_item_purge() {
    let mut ctx = test_context();
    seed_mixed_trash(&mut ctx);
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(&scan_event(mixed_scan()), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('d')), &mut ctx),
        ScreenAction::Stay
    ));

    assert_purge_item_action(
        &screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        &PurgeRequest {
            scope: PurgeScope::Item {
                kind: ItemKind::Note,
                target: TargetSelector::ById(NOTE_ID.to_string()),
            },
            confirm: true,
        },
    );
}

#[test]
fn trash_purge_all_confirm_snapshot() {
    let mut ctx = test_context();
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

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Actions───────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│[j/k] Move  [r] Restore  [R] Restore to path  [d] Purge  [D] Purge expired  [t] Filter: all  [a] Kind: note       │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Trash─────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│Kind   Title or name              Original path              Trashed at            Expires at            Expired  │
│> note  Expired note               notes/expired.md           2026-05-01T12:00:00Z  2026-05-02T12:00:00Z  yes     │
│  asset diagram.png                assets/diagram.png         2026-05-01T12:00:00Z  2999-05-02T12:00:00Z  no      │
│                                                                                                                  │
│                                                                                                                  │
│                 ┌Purge expired─────────────────────────────────────────────────────────────────┐                 │
│                 │Purge 1 expired trash item? Press Enter to confirm or Esc to cancel.          │                 │
│                 └──────────────────────────────────────────────────────────────────────────────┘                 │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
│                                                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
"###);
}

#[test]
fn trash_purge_all_confirm_enqueues_expired_purge() {
    let mut ctx = test_context();
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

    assert_purge_expired_action(
        &screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        &PurgeRequest {
            scope: PurgeScope::Expired,
            confirm: true,
        },
    );
}

#[test]
fn trash_action_kind_toggle_switches_restore_intent() {
    let mut ctx = test_context();
    seed_mixed_trash(&mut ctx);
    let mut screen = TrashScreen::default();
    assert!(matches!(
        screen.handle_event(&scan_event(mixed_scan()), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('a')), &mut ctx),
        ScreenAction::Stay
    ));
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('r')), &mut ctx),
        ScreenAction::Stay
    ));

    assert_restore_asset_action(
        &screen.handle_event(&key(KeyCode::Enter), &mut ctx),
        &RestoreRequest {
            kind: ItemKind::Asset,
            target: TargetSelector::ById(NOTE_ID.to_string()),
            alternate_path: None,
        },
    );
}

#[test]
fn trash_router_delivers_t_to_trash_screen() {
    let mut ctx = test_context();
    seed_mixed_trash(&mut ctx);
    let mut app = App::with_screens(
        vec![
            Box::new(StubScreen::new("Overview Stub")),
            Box::new(StubScreen::new("Notes Stub")),
            Box::new(StubScreen::new("Sync Stub")),
            Box::new(StubScreen::new("Doctor Stub")),
            Box::<TrashScreen>::default(),
        ],
        ctx,
    );
    let mut terminal = test_terminal();

    assert!(app.handle_event(key(KeyCode::Char('5'))));
    assert!(app.handle_event(scan_event(mixed_scan())));
    assert!(app.handle_event(key(KeyCode::Char('t'))));
    draw_app(&mut terminal, &mut app);

    assert!(rendered_rows(&terminal).contains("Filter: expired"));
}

struct StubScreen {
    title: &'static str,
}

impl StubScreen {
    fn new(title: &'static str) -> Self {
        Self { title }
    }
}

impl Screen for StubScreen {
    fn title(&self) -> &str {
        self.title
    }

    fn handle_event(&mut self, _event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        ScreenAction::Stay
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        frame.render_widget(Paragraph::new(self.title), area);
    }
}

fn test_context() -> ScreenCtx {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.vault_root = "/vault".into();
    ctx.config.vault_path = ctx.vault_root.clone();
    ctx.config.trash_dir = ".trash".to_string();
    ctx
}

fn seed_mixed_trash(ctx: &mut ScreenCtx) {
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

fn restored_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::TrashOutcome(
        TrashOutcome::Restored(RestoredItem {
            item_id: NOTE_ID.to_string(),
            kind: ItemKind::Note,
            restored_to: RelativePath::new("notes/expired.md"),
        }),
    ))))
}

fn scan_event(outcome: ScanOutcome) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::ScanOutcome(
        outcome,
    ))))
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn assert_scan_action(action: &ScreenAction) {
    let ScreenAction::EnqueueJob(JobSpec::ScanVault(ScanArgs {
        include_trashed: true,
    })) = action
    else {
        panic!("expected scan-vault enqueue action");
    };
}

fn assert_restore_note_action(action: &ScreenAction, expected: &RestoreRequest) {
    let ScreenAction::EnqueueJob(JobSpec::TrashAction(TrashIntent::RestoreNote(actual))) = action
    else {
        panic!("expected restore-note enqueue action");
    };
    assert_eq!(actual, expected);
}

fn assert_restore_asset_action(action: &ScreenAction, expected: &RestoreRequest) {
    let ScreenAction::EnqueueJob(JobSpec::TrashAction(TrashIntent::RestoreAsset(actual))) = action
    else {
        panic!("expected restore-asset enqueue action");
    };
    assert_eq!(actual, expected);
}

fn assert_purge_item_action(action: &ScreenAction, expected: &PurgeRequest) {
    let ScreenAction::EnqueueJob(JobSpec::TrashAction(TrashIntent::PurgeItem(actual))) = action
    else {
        panic!("expected purge-item enqueue action");
    };
    assert_eq!(actual, expected);
}

fn assert_purge_expired_action(action: &ScreenAction, expected: &PurgeRequest) {
    let ScreenAction::EnqueueJob(JobSpec::TrashAction(TrashIntent::PurgeExpired(actual))) = action
    else {
        panic!("expected purge-expired enqueue action");
    };
    assert_eq!(actual, expected);
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut TrashScreen, ctx: &ScreenCtx) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("trash screen should render to test backend");
}

fn draw_app(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    terminal
        .draw(|frame| app.render(frame))
        .expect("app should render to test backend");
}

fn rendered_rows(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(WIDTH as usize)
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
