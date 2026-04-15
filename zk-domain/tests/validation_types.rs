use serde_json::Value;
use zk_domain::id::RelativePath;
use zk_domain::note::{
    DiagnosticLevel, RuleId, ScanDiagnostic, ValidationFinding, ValidationReport, ValidationSummary,
};

fn all_rule_ids() -> Vec<RuleId> {
    vec![
        RuleId::InvalidFrontmatter,
        RuleId::MissingTitle,
        RuleId::DuplicateId,
        RuleId::EmptyBody,
        RuleId::NoH1InBody,
        RuleId::TitleH1Mismatch,
        RuleId::SuspiciousPathNaming,
        RuleId::MermaidBlockDetected,
        RuleId::TitleDerivedFromH1,
        RuleId::TitleDerivedFromFilename,
        RuleId::IdInjected,
        RuleId::DefaultUntaggedTag,
        RuleId::FrontmatterInserted,
        RuleId::IdFormatSuspicious,
    ]
}

fn description_for(rule: RuleId) -> &'static str {
    match rule {
        RuleId::InvalidFrontmatter => "invalid frontmatter",
        RuleId::MissingTitle => "missing title after derivation",
        RuleId::DuplicateId => "duplicate note id",
        RuleId::EmptyBody => "empty note body",
        RuleId::NoH1InBody => "missing h1 in body",
        RuleId::TitleH1Mismatch => "frontmatter title and h1 mismatch",
        RuleId::SuspiciousPathNaming => "suspicious path naming",
        RuleId::MermaidBlockDetected => "mermaid block detected",
        RuleId::TitleDerivedFromH1 => "title derived from h1",
        RuleId::TitleDerivedFromFilename => "title derived from filename",
        RuleId::IdInjected => "id injected",
        RuleId::DefaultUntaggedTag => "default untagged tag",
        RuleId::FrontmatterInserted => "frontmatter inserted",
        RuleId::IdFormatSuspicious => "suspicious id format",
        _ => panic!("unexpected RuleId variant outside TG5-001 coverage: {rule:?}"),
    }
}

#[test]
fn diagnostic_level_serde_roundtrip_all_variants() {
    let levels = [
        DiagnosticLevel::Error,
        DiagnosticLevel::Warning,
        DiagnosticLevel::Info,
    ];

    for level in levels {
        let json = serde_json::to_string(&level).expect("diagnostic level should serialize");
        let back: DiagnosticLevel =
            serde_json::from_str(&json).expect("diagnostic level should deserialize");
        assert_eq!(back, level);
    }
}

#[test]
fn scan_diagnostic_serde_roundtrip() {
    let diagnostic = ScanDiagnostic {
        level: DiagnosticLevel::Warning,
        path: RelativePath::new("notes/example.md"),
        message: "example message".to_string(),
    };

    let json = serde_json::to_string(&diagnostic).expect("scan diagnostic should serialize");
    let back: ScanDiagnostic =
        serde_json::from_str(&json).expect("scan diagnostic should deserialize");

    assert_eq!(back, diagnostic);
}

#[test]
fn rule_id_serde_roundtrip_all_variants() {
    for rule in all_rule_ids() {
        let json = serde_json::to_string(&rule).expect("rule id should serialize");
        let back: RuleId = serde_json::from_str(&json).expect("rule id should deserialize");
        assert_eq!(back, rule);
    }
}

#[test]
fn rule_id_description_exact_mapping() {
    for rule in all_rule_ids() {
        assert_eq!(rule.description(), description_for(rule));
    }
}

#[test]
fn validation_finding_serde_roundtrip() {
    let finding = ValidationFinding {
        rule: RuleId::MissingTitle,
        level: DiagnosticLevel::Error,
        path: RelativePath::new("notes/untitled.md"),
        message: "missing title after derivation".to_string(),
    };

    let json = serde_json::to_string(&finding).expect("validation finding should serialize");
    let back: ValidationFinding =
        serde_json::from_str(&json).expect("validation finding should deserialize");

    assert_eq!(back, finding);
}

#[test]
fn validation_summary_zero_counts_semantic_json() {
    let summary = ValidationSummary {
        error_count: 0,
        warning_count: 0,
        info_count: 0,
        rules_triggered: Vec::new(),
    };

    let json = serde_json::to_string(&summary).expect("validation summary should serialize");
    let value: Value = serde_json::from_str(&json).expect("summary json should parse");

    assert_eq!(value["error_count"].as_u64(), Some(0));
    assert_eq!(value["warning_count"].as_u64(), Some(0));
    assert_eq!(value["info_count"].as_u64(), Some(0));
    assert_eq!(value["rules_triggered"].as_array().map(Vec::len), Some(0));
}

#[test]
fn validation_summary_roundtrip_with_rules_triggered() {
    let summary = ValidationSummary {
        error_count: 2,
        warning_count: 1,
        info_count: 3,
        rules_triggered: vec![
            (RuleId::MissingTitle, 2),
            (RuleId::EmptyBody, 1),
            (RuleId::MermaidBlockDetected, 3),
        ],
    };

    let json = serde_json::to_string(&summary).expect("validation summary should serialize");
    let back: ValidationSummary =
        serde_json::from_str(&json).expect("validation summary should deserialize");

    assert_eq!(back, summary);
}

#[test]
fn validation_report_roundtrip_preserves_all_sections() {
    let report = ValidationReport {
        findings: vec![
            ValidationFinding {
                rule: RuleId::MissingTitle,
                level: DiagnosticLevel::Error,
                path: RelativePath::new("notes/a.md"),
                message: "missing title after derivation".to_string(),
            },
            ValidationFinding {
                rule: RuleId::EmptyBody,
                level: DiagnosticLevel::Warning,
                path: RelativePath::new("notes/b.md"),
                message: "empty note body".to_string(),
            },
        ],
        summary: ValidationSummary {
            error_count: 1,
            warning_count: 1,
            info_count: 0,
            rules_triggered: vec![(RuleId::MissingTitle, 1), (RuleId::EmptyBody, 1)],
        },
        scan_diagnostics: vec![ScanDiagnostic {
            level: DiagnosticLevel::Error,
            path: RelativePath::new("notes/c.md"),
            message: "failed to parse note during vault scan: yaml parse error".to_string(),
        }],
    };

    let json = serde_json::to_string(&report).expect("validation report should serialize");
    let back: ValidationReport =
        serde_json::from_str(&json).expect("validation report should deserialize");

    assert_eq!(back, report);
}

#[test]
fn validation_report_semantic_json_shape() {
    let report = ValidationReport {
        findings: vec![ValidationFinding {
            rule: RuleId::IdInjected,
            level: DiagnosticLevel::Info,
            path: RelativePath::new("notes/d.md"),
            message: "injected missing id into frontmatter".to_string(),
        }],
        summary: ValidationSummary {
            error_count: 0,
            warning_count: 0,
            info_count: 1,
            rules_triggered: vec![(RuleId::IdInjected, 1)],
        },
        scan_diagnostics: vec![ScanDiagnostic {
            level: DiagnosticLevel::Info,
            path: RelativePath::new("notes/d.md"),
            message: "injected missing id into frontmatter".to_string(),
        }],
    };

    let json = serde_json::to_string(&report).expect("validation report should serialize");
    let value: Value = serde_json::from_str(&json).expect("report json should parse");

    assert!(value.get("findings").is_some());
    assert!(value.get("summary").is_some());
    assert!(value.get("scan_diagnostics").is_some());

    assert_eq!(value["findings"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["summary"]["error_count"].as_u64(), Some(0));
    assert_eq!(value["summary"]["warning_count"].as_u64(), Some(0));
    assert_eq!(value["summary"]["info_count"].as_u64(), Some(1));
    assert_eq!(value["scan_diagnostics"].as_array().map(Vec::len), Some(1));
}
