use serde_json::Value;
use zk_domain::id::RelativePath;
use zk_domain::note::{
    DiagnosticLevel, RuleId, ScanDiagnostic, ValidationFinding, ValidationReport, ValidationSummary,
};

fn sample_report() -> ValidationReport {
    ValidationReport {
        findings: vec![
            ValidationFinding {
                rule: RuleId::MissingTitle,
                level: DiagnosticLevel::Error,
                path: RelativePath::new("notes/error.md"),
                message: "missing title".to_string(),
            },
            ValidationFinding {
                rule: RuleId::EmptyBody,
                level: DiagnosticLevel::Warning,
                path: RelativePath::new("notes/warning.md"),
                message: "empty body".to_string(),
            },
            ValidationFinding {
                rule: RuleId::MermaidBlockDetected,
                level: DiagnosticLevel::Info,
                path: RelativePath::new("notes/info.md"),
                message: "mermaid detected".to_string(),
            },
        ],
        summary: ValidationSummary {
            error_count: 1,
            warning_count: 1,
            info_count: 1,
            rules_triggered: vec![
                (RuleId::MissingTitle, 1),
                (RuleId::EmptyBody, 1),
                (RuleId::MermaidBlockDetected, 1),
            ],
        },
        scan_diagnostics: vec![ScanDiagnostic {
            level: DiagnosticLevel::Warning,
            path: RelativePath::new("notes/warning.md"),
            message: "legacy warning".to_string(),
        }],
    }
}

#[test]
fn to_json_pretty_returns_valid_json_with_expected_shape() {
    let report = sample_report();
    let pretty = report.to_json();
    let value: Value = serde_json::from_str(&pretty).expect("pretty json should parse");

    assert!(pretty.contains('\n'));
    assert_eq!(value["summary"]["error_count"].as_u64(), Some(1));
    assert_eq!(value["summary"]["warning_count"].as_u64(), Some(1));
    assert_eq!(value["summary"]["info_count"].as_u64(), Some(1));
    assert_eq!(value["findings"].as_array().map(Vec::len), Some(3));
    assert_eq!(value["scan_diagnostics"].as_array().map(Vec::len), Some(1));
}

#[test]
fn to_json_compact_returns_valid_json_equivalent_to_pretty() {
    let report = sample_report();
    let compact = report.to_json_compact();
    let pretty = report.to_json();

    assert!(!compact.contains('\n'));

    let compact_value: Value =
        serde_json::from_str(&compact).expect("compact json should parse successfully");
    let pretty_value: Value =
        serde_json::from_str(&pretty).expect("pretty json should parse successfully");
    assert_eq!(compact_value, pretty_value);
}

#[test]
fn empty_report_json_contains_zero_counts() {
    let report = ValidationReport {
        findings: Vec::new(),
        summary: ValidationSummary {
            error_count: 0,
            warning_count: 0,
            info_count: 0,
            rules_triggered: Vec::new(),
        },
        scan_diagnostics: Vec::new(),
    };

    let pretty = report.to_json();
    let compact = report.to_json_compact();
    let pretty_value: Value = serde_json::from_str(&pretty).expect("pretty json should parse");
    let compact_value: Value = serde_json::from_str(&compact).expect("compact json should parse");

    assert_eq!(pretty_value["summary"]["error_count"].as_u64(), Some(0));
    assert_eq!(pretty_value["summary"]["warning_count"].as_u64(), Some(0));
    assert_eq!(pretty_value["summary"]["info_count"].as_u64(), Some(0));
    assert_eq!(compact_value["summary"]["error_count"].as_u64(), Some(0));
    assert_eq!(compact_value["summary"]["warning_count"].as_u64(), Some(0));
    assert_eq!(compact_value["summary"]["info_count"].as_u64(), Some(0));
}
