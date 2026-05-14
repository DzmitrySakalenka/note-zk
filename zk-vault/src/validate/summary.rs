use std::collections::HashMap;

use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding, ValidationSummary};

pub(super) fn build_summary(findings: &[ValidationFinding]) -> ValidationSummary {
    let mut error_count: u32 = 0;
    let mut warning_count: u32 = 0;
    let mut info_count: u32 = 0;
    let mut by_rule: HashMap<RuleId, u32> = HashMap::new();

    for finding in findings {
        match finding.level {
            DiagnosticLevel::Error => error_count += 1,
            DiagnosticLevel::Warning => warning_count += 1,
            DiagnosticLevel::Info => info_count += 1,
        }

        let count = by_rule.entry(finding.rule).or_insert(0);
        *count += 1;
    }

    let mut rules_triggered: Vec<(RuleId, u32)> = by_rule.into_iter().collect();
    rules_triggered.sort_by_key(|(rule, _)| rule_sort_key(*rule));

    ValidationSummary {
        error_count,
        warning_count,
        info_count,
        rules_triggered,
    }
}

fn rule_sort_key(rule: RuleId) -> u8 {
    match rule {
        RuleId::InvalidFrontmatter => 0,
        RuleId::MissingTitle => 1,
        RuleId::DuplicateId => 2,
        RuleId::EmptyBody => 3,
        RuleId::NoH1InBody => 4,
        RuleId::TitleH1Mismatch => 5,
        RuleId::SuspiciousPathNaming => 6,
        RuleId::MermaidBlockDetected => 7,
        RuleId::TitleDerivedFromH1 => 8,
        RuleId::TitleDerivedFromFilename => 9,
        RuleId::IdInjected => 10,
        RuleId::DefaultUntaggedTag => 11,
        RuleId::FrontmatterInserted => 12,
        RuleId::IdFormatSuspicious => 13,
        RuleId::TrashedUnknownIdentity => 14,
        RuleId::UnresolvedLink => 15,
        RuleId::AmbiguousLink => 16,
        RuleId::BrokenAssetRef => 17,
        RuleId::WikiUnknownType => 18,
        RuleId::WikiSourceMissing => 19,
        RuleId::WikiIndexOutsideWikiRoot => 20,
        _ => u8::MAX,
    }
}
