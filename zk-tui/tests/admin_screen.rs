#![allow(unused_crate_dependencies)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use ratatui::{Terminal, backend::TestBackend};
use zk_cli_core::admin::{
    BackupArchiveDescriptor, BackupManifest, BackupManifestEncryption, BackupSourceSummary,
    CancelResponse, ListBackupsResponse, ListedBackup, ListedJob, RecomputeResponse,
};
use zk_tui::{
    app::{Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, BackgroundResult},
    jobs::{JobKind, JobResult, JobSpec},
    screens::admin::AdminScreen,
};

const WIDTH: u16 = 110;
const HEIGHT: u16 = 20;
const BACKUP_ID: &str = "01HZZZZZZZZZZZZZZZZZZZZZZZ";
const JOB_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";

#[test]
fn admin_missing_token_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    assert!(matches!(
        screen.handle_event(
            &job_error_event(
                JobKind::AdminBackupList,
                "environment variable ZK_DEVICE_TOKEN is required for admin screen"
            ),
            &mut ctx,
        ),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Admin───────────────────────────────────────────────────────────────────────────────────────────────────────┐
│Tabs: >[1] Backups<  [2] AI Jobs                                                                            │
│Server: https://zk.example.test/  Token env: ZK_DEVICE_TOKEN                                                │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Backups────────────────────────────────────────────────────┐┌Backup detail──────────────────────────────────┐
│Error: Missing token: set $ZK_DEVICE_TOKEN                 ││Enter shows manifest detail.                   │
│                                                           ││                                               │
│                                                           ││Status: idle                                   │
│                                                           ││[Enter] Detail  [T] Trigger backup  [r] Refresh│
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
└───────────────────────────────────────────────────────────┘└───────────────────────────────────────────────┘
"###);
}

#[test]
fn admin_populated_backups_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    screen.handle_event(&backup_list_event(), &mut ctx);
    screen.handle_event(&manifest_event(), &mut ctx);

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Admin───────────────────────────────────────────────────────────────────────────────────────────────────────┐
│Tabs: >[1] Backups<  [2] AI Jobs                                                                            │
│Server: https://zk.example.test/  Token env: ZK_DEVICE_TOKEN                                                │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Backups────────────────────────────────────────────────────┐┌Backup detail──────────────────────────────────┐
│BACKUP_ID              CREATED_AT             SIZE  ENC    ││backup_id: 01HZZZZZZZZZZZZZZZZZZZZZZZ          │
│> 01HZZZZZZZZZZZZZZZZZ  2026-05-02T10:00:00Z   128  none   ││schema_version: 1                              │
│                                                           ││created_at: 2026-05-02T10:00:00Z               │
│in_flight: manual-run                                      ││archive: backup-01HZZZZZZZZZZZZZZZZZZZZZZZ.tar.│
│                                                           ││sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa│
│                                                           ││size_bytes: 128                                │
│                                                           ││encryption: none                               │
│                                                           ││retention: not recorded in manifest            │
│                                                           ││source_vault_path: /vault                      │
│                                                           ││note_count: 3                                  │
│                                                           ││asset_count: 2                                 │
│                                                           ││server_db_included: true                       │
│                                                           ││server_db_size_bytes: 4096                     │
│                                                           ││                                               │
└───────────────────────────────────────────────────────────┘└───────────────────────────────────────────────┘
"###);
}

#[test]
fn admin_populated_ai_jobs_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);
    screen.handle_event(&ai_jobs_event(), &mut ctx);

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Admin───────────────────────────────────────────────────────────────────────────────────────────────────────┐
│Tabs: [1] Backups  >[2] AI Jobs<                                                                            │
│Server: https://zk.example.test/  Token env: ZK_DEVICE_TOKEN                                                │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌AI Jobs───────────────────────────────────────────────────────────┐┌Job detail──────────────────────────────┐
│JOB_ID        KIND                STATUS     ATT  ENQUEUED        ││job_id: 01JAR5A3PJEZ3SKG67PN8N6S6S      │
│> 01JAR5A3PJEZ  recompute_vault      pending      0  2026-05-04T12││kind: recompute_vault                   │
│  01JAR5A3PJEZ  note[01JAR5A3]       failed       3  2026-05-04T12││status: pending                         │
│                                                                  ││attempts: 0                             │
│                                                                  ││enqueued_at: 2026-05-04T12:34:56Z       │
│                                                                  ││started_at: none                        │
│                                                                  ││finished_at: none                       │
│                                                                  ││last_error: none                        │
│                                                                  ││                                        │
│                                                                  ││Status: idle                            │
│                                                                  ││[c] Cancel pending  [R] Recompute vault │
│                                                                  ││                                        │
│                                                                  ││                                        │
│                                                                  ││                                        │
└──────────────────────────────────────────────────────────────────┘└────────────────────────────────────────┘
"###);
}

#[test]
fn admin_trigger_backup_sets_in_flight_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    screen.handle_event(&backup_list_event(), &mut ctx);

    let action = screen.handle_event(&key(KeyCode::Char('T')), &mut ctx);
    assert!(matches!(
        action,
        ScreenAction::EnqueueJob(JobSpec::AdminBackupTrigger)
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Admin───────────────────────────────────────────────────────────────────────────────────────────────────────┐
│Tabs: >[1] Backups<  [2] AI Jobs                                                                            │
│Server: https://zk.example.test/  Token env: ZK_DEVICE_TOKEN                                                │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌Backups────────────────────────────────────────────────────┐┌Backup detail──────────────────────────────────┐
│BACKUP_ID              CREATED_AT             SIZE  ENC    ││Enter shows manifest detail.                   │
│> 01HZZZZZZZZZZZZZZZZZ  2026-05-02T10:00:00Z   128  none   ││                                               │
│                                                           ││Status: triggering backup...                   │
│in_flight: manual-run                                      ││[Enter] Detail  [T] Trigger backup  [r] Refresh│
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
│                                                           ││                                               │
└───────────────────────────────────────────────────────────┘└───────────────────────────────────────────────┘
"###);
}

#[test]
fn admin_cancel_confirm_snapshot_and_action() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);
    screen.handle_event(&ai_jobs_event(), &mut ctx);
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('c')), &mut ctx),
        ScreenAction::Stay
    ));

    let mut terminal = test_terminal();
    draw_screen(&mut terminal, &mut screen, &ctx);

    assert_snapshot!(rendered_rows(&terminal), @r###"
┌Admin───────────────────────────────────────────────────────────────────────────────────────────────────────┐
│Tabs: [1] Backups  >[2] AI Jobs<                                                                            │
│Server: https://zk.example.test/  Token env: ZK_DEVICE_TOKEN                                                │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
┌AI Jobs───────────────────────────────────────────────────────────┐┌Job detail──────────────────────────────┐
│JOB_ID        KIND                STATUS     ATT  ENQUEUED        ││job_id: 01JAR5A3PJEZ3SKG67PN8N6S6S      │
│> 01JAR5A3PJEZ  recompute_vault      pending      0  2026-05-04T12││kind: recompute_vault                   │
│  01JAR5A3PJE┌Confirm─────────────────────────────────────────────────────────────────────────┐             │
│             │Cancel AI job 01JAR5A3PJEZ3SKG67PN8N6S6S?                                       │             │
│             │Press Enter to confirm or Esc to cancel.                                        │34:56Z       │
│             │                                                                                │             │
│             └────────────────────────────────────────────────────────────────────────────────┘             │
│                                                                  ││last_error: none                        │
│                                                                  ││                                        │
│                                                                  ││Status: idle                            │
│                                                                  ││[c] Cancel pending  [R] Recompute vault │
│                                                                  ││                                        │
│                                                                  ││                                        │
│                                                                  ││                                        │
└──────────────────────────────────────────────────────────────────┘└────────────────────────────────────────┘
"###);

    let action = screen.handle_event(&key(KeyCode::Enter), &mut ctx);
    let ScreenAction::EnqueueJob(JobSpec::AdminAiCancel { job_id }) = action else {
        panic!("expected admin AI cancel job");
    };
    assert_eq!(job_id, JOB_ID);
}

#[test]
fn admin_recompute_vault_action_refreshes_ai_jobs() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);

    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('R')), &mut ctx),
        ScreenAction::EnqueueJob(JobSpec::AdminAiRecomputeVault)
    ));
    assert!(matches!(
        screen.handle_event(
            &AppEvent::Background(BackgroundResult::new(Box::new(
                JobResult::AdminAiRecomputed(RecomputeResponse {
                    job_id: JOB_ID.to_string(),
                    status: "pending".to_string(),
                },)
            ))),
            &mut ctx,
        ),
        ScreenAction::EnqueueJob(JobSpec::AdminAiJobsList {
            status: None,
            limit: None,
        })
    ));
}

#[test]
fn admin_cancel_success_refreshes_ai_jobs() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);

    assert!(matches!(
        screen.handle_event(
            &AppEvent::Background(BackgroundResult::new(Box::new(
                JobResult::AdminAiCancelled {
                    job_id: JOB_ID.to_string(),
                    response: CancelResponse { cancelled: true },
                }
            ))),
            &mut ctx,
        ),
        ScreenAction::EnqueueJob(JobSpec::AdminAiJobsList {
            status: None,
            limit: None,
        })
    ));
}

fn backup_list_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::AdminBackups(
        ListBackupsResponse {
            items: vec![ListedBackup {
                backup_id: BACKUP_ID.to_string(),
                created_at: "2026-05-02T10:00:00Z".to_string(),
                size_bytes: 128,
                encryption_type: "none".to_string(),
            }],
            in_flight: vec!["manual-run".to_string()],
        },
    ))))
}

fn manifest_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(
        JobResult::AdminBackupManifest(manifest()),
    )))
}

fn ai_jobs_event() -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::AdminAiJobs(
        vec![
            ListedJob {
                job_id: JOB_ID.to_string(),
                kind: "recompute_vault".to_string(),
                note_id: None,
                status: "pending".to_string(),
                attempts: 0,
                last_error: None,
                enqueued_at: "2026-05-04T12:34:56Z".to_string(),
                started_at: None,
                finished_at: None,
            },
            ListedJob {
                job_id: "01JAR5A3PJEZ3SKG67PN8N6S6T".to_string(),
                kind: "recompute_note".to_string(),
                note_id: Some("01JAR5A3PJEZ3SKG67PN8N6S6R".to_string()),
                status: "failed".to_string(),
                attempts: 3,
                last_error: Some("provider unavailable".to_string()),
                enqueued_at: "2026-05-04T12:30:00Z".to_string(),
                started_at: Some("2026-05-04T12:31:00Z".to_string()),
                finished_at: Some("2026-05-04T12:32:00Z".to_string()),
            },
        ],
    ))))
}

fn job_error_event(kind: JobKind, message: &str) -> AppEvent {
    AppEvent::Background(BackgroundResult::new(Box::new(JobResult::Error {
        kind,
        error: anyhow::anyhow!(message.to_string()),
    })))
}

fn manifest() -> BackupManifest {
    BackupManifest {
        schema_version: 1,
        backup_id: BACKUP_ID.to_string(),
        created_at: "2026-05-02T10:00:00Z".to_string(),
        source_summary: BackupSourceSummary {
            vault_path: "/vault".into(),
            note_count: 3,
            asset_count: 2,
            server_db_included: true,
            server_db_size_bytes: Some(4096),
        },
        archive: BackupArchiveDescriptor {
            file_name: format!("backup-{BACKUP_ID}.tar.zst"),
            size_bytes: 128,
            sha256_hex: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        },
        encryption: BackupManifestEncryption::None,
    }
}

fn configured_context() -> ScreenCtx {
    let mut ctx = ScreenCtx::for_test().expect("screen context should initialize");
    ctx.config.sync.server_url = Some(
        "https://zk.example.test/"
            .parse()
            .expect("server URL should parse"),
    );
    ctx
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw_screen(terminal: &mut Terminal<TestBackend>, screen: &mut AdminScreen, ctx: &ScreenCtx) {
    terminal
        .draw(|frame| screen.render(frame, frame.area(), ctx))
        .expect("screen should render to test backend");
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
        })
        .collect::<Vec<_>>()
        .join("\n")
}
