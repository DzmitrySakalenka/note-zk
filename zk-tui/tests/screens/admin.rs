use crossterm::event::KeyCode;
use zk_cli_core::admin::{
    BackupArchiveDescriptor, BackupManifest, BackupManifestEncryption, BackupSourceSummary,
    ListBackupsResponse, ListedBackup, ListedJob,
};
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{JobKind, JobResult, JobSpec},
    screens::admin::AdminScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, configured_context, job_error_event, key, render_screen,
};

const BACKUP_ID: &str = "01HZZZZZZZZZZZZZZZZZZZZZZZ";
const JOB_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";

#[test]
fn admin_error_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    assert!(matches!(
        screen.handle_event(
            &job_error_event(
                JobKind::AdminBackupList,
                "environment variable ZK_DEVICE_TOKEN is required for admin screen"
            ),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__admin__error", &render_screen(&mut screen, &ctx));
}

#[test]
fn admin_backups_loaded_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    let _ = screen.handle_event(&backup_list_event(), &mut ctx);
    let _ = screen.handle_event(&manifest_event(), &mut ctx);

    assert_screen_snapshot(
        "screens__admin__backups_loaded",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn admin_backup_in_flight_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    let _ = screen.handle_event(&backup_list_event(), &mut ctx);
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Char('T')), &mut ctx),
        ScreenAction::EnqueueJob(JobSpec::AdminBackupTrigger)
    ));

    assert_screen_snapshot(
        "screens__admin__backup_in_flight",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn admin_ai_jobs_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AdminScreen::default();
    let _ = screen.handle_event(&key(KeyCode::Char('2')), &mut ctx);
    let _ = screen.handle_event(&ai_jobs_event(), &mut ctx);

    assert_screen_snapshot("screens__admin__ai_jobs", &render_screen(&mut screen, &ctx));
}

fn backup_list_event() -> zk_tui::event::AppEvent {
    background(JobResult::AdminBackups(ListBackupsResponse {
        items: vec![ListedBackup {
            backup_id: BACKUP_ID.to_string(),
            created_at: "2026-05-02T10:00:00Z".to_string(),
            size_bytes: 128,
            encryption_type: "none".to_string(),
        }],
        in_flight: vec!["manual-run".to_string()],
    }))
}

fn manifest_event() -> zk_tui::event::AppEvent {
    background(JobResult::AdminBackupManifest(manifest()))
}

fn ai_jobs_event() -> zk_tui::event::AppEvent {
    background(JobResult::AdminAiJobs(vec![
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
    ]))
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
