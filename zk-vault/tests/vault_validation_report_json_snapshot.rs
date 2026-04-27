// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use insta::assert_snapshot;
use zk_domain::{
    id::RelativePath,
    note::{
        DiagnosticLevel, RuleId, ScanDiagnostic, ValidationFinding, ValidationReport,
        ValidationSummary,
    },
};

fn mixed_report() -> ValidationReport {
    ValidationReport {
        findings: vec![
            ValidationFinding {
                rule: RuleId::MissingTitle,
                level: DiagnosticLevel::Error,
                path: RelativePath::new("notes/a.md"),
                message: "missing title".to_string(),
            },
            ValidationFinding {
                rule: RuleId::EmptyBody,
                level: DiagnosticLevel::Warning,
                path: RelativePath::new("notes/b.md"),
                message: "empty body".to_string(),
            },
            ValidationFinding {
                rule: RuleId::IdInjected,
                level: DiagnosticLevel::Info,
                path: RelativePath::new("notes/c.md"),
                message: "id injected".to_string(),
            },
        ],
        summary: ValidationSummary {
            error_count: 1,
            warning_count: 1,
            info_count: 1,
            rules_triggered: vec![
                (RuleId::MissingTitle, 1),
                (RuleId::EmptyBody, 1),
                (RuleId::IdInjected, 1),
            ],
        },
        scan_diagnostics: vec![ScanDiagnostic {
            level: DiagnosticLevel::Warning,
            path: RelativePath::new("notes/b.md"),
            message: "legacy warning".to_string(),
        }],
    }
}

#[test]
fn snapshot_validation_report_json_mixed_levels() {
    let report = mixed_report();
    assert_snapshot!(
        "validation_report_json_mixed_levels",
        report
            .to_json()
            .expect("validation report JSON snapshot should serialize")
    );
}
