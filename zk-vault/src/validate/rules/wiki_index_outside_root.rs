use zk_domain::note::{DiagnosticLevel, RuleId, ValidationFinding};

use crate::NormalizationResult;

use super::super::{
    ValidationContext, ValidationRule,
    support::{is_wiki_meta_type, new_finding, path_has_prefix},
};

pub(crate) const ID: RuleId = RuleId::WikiIndexOutsideWikiRoot;

pub(crate) struct Rule;

impl ValidationRule for Rule {
    fn id(&self) -> RuleId {
        ID
    }

    fn default_level(&self) -> DiagnosticLevel {
        DiagnosticLevel::Error
    }

    fn check_note(
        &self,
        note: &NormalizationResult,
        ctx: &ValidationContext<'_>,
    ) -> Vec<ValidationFinding> {
        let Some(wiki) = ctx.config.wiki.as_ref() else {
            return Vec::new();
        };
        let Some(note_type) = note
            .frontmatter
            .as_ref()
            .and_then(|frontmatter| frontmatter.note_type.as_deref())
        else {
            return Vec::new();
        };
        if !is_wiki_meta_type(note_type) || path_has_prefix(&note.relative_path, &wiki.root_dir) {
            return Vec::new();
        }

        vec![new_finding(
            ID,
            DiagnosticLevel::Error,
            note.relative_path.clone(),
            "wiki index/log page outside wiki root",
        )]
    }
}
