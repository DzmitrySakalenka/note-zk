use insta::assert_snapshot;
use zk_app::{DoctorIssue, DoctorReport, StatusSummary};
use zk_cli::output::{
    ScanCommandPayload, ScanSummary, write_doctor_human, write_scan_human, write_status_human,
};
use zk_domain::{id::RelativePath, note::DiagnosticLevel, sync::ItemType};
use zk_vault::NormalizationSummary;

fn render_scan(payload: &ScanCommandPayload) -> String {
    let mut output = Vec::new();
    write_scan_human(&mut output, payload).expect("scan formatter should write");
    String::from_utf8(output).expect("scan formatter should emit UTF-8")
}

fn render_status(summary: &StatusSummary) -> String {
    let mut output = Vec::new();
    write_status_human(&mut output, summary).expect("status formatter should write");
    String::from_utf8(output).expect("status formatter should emit UTF-8")
}

fn render_doctor(report: &DoctorReport) -> String {
    let mut output = Vec::new();
    write_doctor_human(&mut output, report).expect("doctor formatter should write");
    String::from_utf8(output).expect("doctor formatter should emit UTF-8")
}

fn scan_check_fixture() -> ScanCommandPayload {
    ScanCommandPayload::Check {
        summary: ScanSummary {
            notes_scanned: 123,
            assets_scanned: 4,
            errors: 12,
            warnings: 3,
            infos: 1,
            duplicate_ids: 2,
            unresolved_links: 45,
        },
    }
}

fn scan_write_fixture() -> ScanCommandPayload {
    ScanCommandPayload::Write {
        summary: ScanSummary {
            notes_scanned: 88,
            assets_scanned: 7,
            errors: 9,
            warnings: 10,
            infos: 11,
            duplicate_ids: 12,
            unresolved_links: 13,
        },
        normalization: NormalizationSummary {
            files_written: 5,
            files_skipped_immutable: 1,
            files_skipped_no_change: 22,
            files_skipped_duplicate_id: 0,
            files_errored: 333,
            diagnostics: Vec::new(),
        },
    }
}

fn status_fixture() -> StatusSummary {
    StatusSummary {
        total_notes: 12,
        total_assets: 3,
        notes_with_errors: 1,
        notes_with_warnings: 99,
        unresolved_link_count: 4,
        duplicate_id_count: 7,
        trashed_notes: 2,
        trashed_assets: 5,
        pending_purge: 6,
        purged_notes: 8,
        purged_assets: 9,
    }
}

fn doctor_fixture() -> DoctorReport {
    DoctorReport {
        errors: vec![DoctorIssue {
            rule_id: "duplicate-id".to_string(),
            path: Some(RelativePath::new("notes/dup-a.md")),
            item_id: Some("01SNAPSHOTDUPLICATE000000001".to_string()),
            item_type: Some(ItemType::Note),
            level: DiagnosticLevel::Error,
            message: "duplicate note id detected".to_string(),
        }],
        warnings: vec![DoctorIssue {
            rule_id: "trash-file-without-record".to_string(),
            path: Some(RelativePath::new("99_trash/issues.md")),
            item_id: Some("01SNAPSHOTTRASHRULE000000001".to_string()),
            item_type: Some(ItemType::Note),
            level: DiagnosticLevel::Warning,
            message: "trashed note file has no matching delete record".to_string(),
        }],
        infos: vec![DoctorIssue {
            rule_id: "retention-expired-not-purged".to_string(),
            path: Some(RelativePath::new("99_trash/expired.md")),
            item_id: Some("01SNAPSHOTRETENTION00000001".to_string()),
            item_type: Some(ItemType::Note),
            level: DiagnosticLevel::Info,
            message:
                "retention expired on 2026-04-01T00:00:00Z; run `zk-cli purge --expired --yes`"
                    .to_string(),
        }],
    }
}

fn doctor_warnings_only_fixture() -> DoctorReport {
    DoctorReport {
        errors: Vec::new(),
        warnings: vec![DoctorIssue {
            rule_id: "unresolved-link".to_string(),
            path: Some(RelativePath::new("notes/issues.md")),
            item_id: None,
            item_type: None,
            level: DiagnosticLevel::Warning,
            message: "wiki link points to a missing note".to_string(),
        }],
        infos: Vec::new(),
    }
}

fn doctor_errors_and_infos_fixture() -> DoctorReport {
    DoctorReport {
        errors: vec![DoctorIssue {
            rule_id: "duplicate-id".to_string(),
            path: Some(RelativePath::new("notes/dup-a.md")),
            item_id: None,
            item_type: None,
            level: DiagnosticLevel::Error,
            message: "duplicate note id detected".to_string(),
        }],
        warnings: Vec::new(),
        infos: vec![DoctorIssue {
            rule_id: "title-derived-from-h1".to_string(),
            path: None,
            item_id: None,
            item_type: None,
            level: DiagnosticLevel::Info,
            message: "title derived from the first heading".to_string(),
        }],
    }
}

#[test]
fn scan_check_human_snapshot() {
    assert_snapshot!("scan_check_human", render_scan(&scan_check_fixture()));
}

#[test]
fn scan_write_human_snapshot() {
    assert_snapshot!("scan_write_human", render_scan(&scan_write_fixture()));
}

#[test]
fn status_human_snapshot() {
    assert_snapshot!("status_human", render_status(&status_fixture()));
}

#[test]
fn doctor_human_snapshot() {
    assert_snapshot!("doctor_human", render_doctor(&doctor_fixture()));
}

#[test]
fn doctor_human_warnings_only_snapshot() {
    assert_snapshot!(
        "doctor_human_warnings_only",
        render_doctor(&doctor_warnings_only_fixture())
    );
}

#[test]
fn doctor_human_errors_and_infos_snapshot() {
    assert_snapshot!(
        "doctor_human_errors_and_infos",
        render_doctor(&doctor_errors_and_infos_fixture())
    );
}

#[test]
fn doctor_human_no_issues_snapshot() {
    assert_snapshot!(
        "doctor_human_no_issues",
        render_doctor(&DoctorReport {
            errors: Vec::new(),
            warnings: Vec::new(),
            infos: Vec::new(),
        })
    );
}
