#[cfg(feature = "ai")]
mod facade;
mod provenance;

#[cfg(feature = "ai")]
pub use facade::{
    AiJobStatus, LocalWikiIndexHandle, WikiFacade, WikiFacadeDeps, WikiJobStatus, WikiLintFinding,
    WikiLintFindingKind, WikiLintScope, WikiLintSeverity,
};
pub use provenance::{list_pages_using_source, list_sources_for_note};
