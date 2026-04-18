use zk_domain::{
    config::EffectiveConfig,
    id::RelativePath,
    note::{DiagnosticLevel, ScanDiagnostic},
};
use zk_vault::{VaultScanResult, validate_vault};

fn base_config() -> EffectiveConfig {
    EffectiveConfig::default()
}

fn empty_scan_result() -> VaultScanResult {
    VaultScanResult {
        notes: Vec::new(),
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
    }
}

#[test]
fn validate_vault_empty_scan_returns_empty_report() {
    let scan_result = empty_scan_result();
    let config = base_config();

    let report = validate_vault(&scan_result, &config);

    assert!(
        report.findings.is_empty(),
        "expected no findings for empty scan"
    );
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.info_count, 0);
    assert!(
        report.summary.rules_triggered.is_empty(),
        "expected no triggered rules for empty scan"
    );
    assert!(
        report.scan_diagnostics.is_empty(),
        "expected no scan diagnostics for empty scan"
    );
}

#[test]
fn validate_vault_passes_scan_diagnostics_through_unchanged() {
    let diagnostics = vec![
        ScanDiagnostic {
            level: DiagnosticLevel::Error,
            path: RelativePath::new("notes/broken-frontmatter.md"),
            message: "failed to parse note during vault scan: yaml parse error".to_string(),
        },
        ScanDiagnostic {
            level: DiagnosticLevel::Warning,
            path: RelativePath::new("notes/no-frontmatter.md"),
            message: "cannot insert frontmatter because allow_insert_frontmatter=false".to_string(),
        },
    ];
    let scan_result = VaultScanResult {
        notes: Vec::new(),
        assets: Vec::new(),
        diagnostics: diagnostics.clone(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
    };
    let config = base_config();

    let report = validate_vault(&scan_result, &config);

    assert_eq!(report.scan_diagnostics, diagnostics);
}
