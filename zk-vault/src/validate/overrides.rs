use std::collections::{BTreeMap, HashMap, HashSet};

use zk_domain::note::{DiagnosticLevel, RuleId};

pub(super) fn parse_disabled_rules(names: &[String]) -> HashSet<RuleId> {
    let mut disabled = HashSet::new();
    for name in names {
        if let Some(rule_id) = parse_rule_id(name) {
            let _ = disabled.insert(rule_id);
        }
    }
    disabled
}

pub(super) fn parse_severity_overrides(
    overrides: &BTreeMap<String, String>,
) -> HashMap<RuleId, DiagnosticLevel> {
    let mut parsed = HashMap::new();
    for (rule_name, level_name) in overrides {
        let Some(rule_id) = parse_rule_id(rule_name) else {
            continue;
        };
        let Some(level) = parse_level_override(level_name) else {
            continue;
        };
        let _ = parsed.insert(rule_id, level);
    }
    parsed
}

fn parse_rule_id(name: &str) -> Option<RuleId> {
    match name {
        "InvalidFrontmatter" => Some(RuleId::InvalidFrontmatter),
        "MissingTitle" => Some(RuleId::MissingTitle),
        "DuplicateId" => Some(RuleId::DuplicateId),
        "EmptyBody" => Some(RuleId::EmptyBody),
        "NoH1InBody" => Some(RuleId::NoH1InBody),
        "TitleH1Mismatch" => Some(RuleId::TitleH1Mismatch),
        "SuspiciousPathNaming" => Some(RuleId::SuspiciousPathNaming),
        "MermaidBlockDetected" => Some(RuleId::MermaidBlockDetected),
        "TitleDerivedFromH1" => Some(RuleId::TitleDerivedFromH1),
        "TitleDerivedFromFilename" => Some(RuleId::TitleDerivedFromFilename),
        "IdInjected" => Some(RuleId::IdInjected),
        "DefaultUntaggedTag" => Some(RuleId::DefaultUntaggedTag),
        "FrontmatterInserted" => Some(RuleId::FrontmatterInserted),
        "IdFormatSuspicious" => Some(RuleId::IdFormatSuspicious),
        "TrashedUnknownIdentity" => Some(RuleId::TrashedUnknownIdentity),
        "UnresolvedLink" => Some(RuleId::UnresolvedLink),
        "AmbiguousLink" => Some(RuleId::AmbiguousLink),
        "BrokenAssetRef" => Some(RuleId::BrokenAssetRef),
        _ => None,
    }
}

fn parse_level_override(name: &str) -> Option<DiagnosticLevel> {
    if name.eq_ignore_ascii_case("error") {
        return Some(DiagnosticLevel::Error);
    }
    if name.eq_ignore_ascii_case("warning") {
        return Some(DiagnosticLevel::Warning);
    }
    if name.eq_ignore_ascii_case("info") {
        return Some(DiagnosticLevel::Info);
    }
    None
}
