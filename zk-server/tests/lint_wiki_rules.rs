// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{collections::BTreeMap, fmt::Write as _, fs, path::Path};

use insta::assert_snapshot;
use rusqlite::Connection;
use tempfile::TempDir;
use zk_domain::{
    config::{EffectiveConfig, EffectiveWikiConfig},
    derived::ResolvedNoteLink,
    id::{ContentHash, NoteId, RelativePath},
    note::NoteFrontmatter,
};
use zk_index::{LinkRepo, NoteRepo, SourcesRepo, UpsertNoteParams};
use zk_server::{
    ai::lint_wiki::{LintWikiService, WikiLintFinding},
    repo::WikiLintScope,
};

#[test]
fn orphan_page_ignores_index_linked_pages() {
    let fixture = LintFixture::new();
    fixture.add_note(NoteSpec::wiki_entity(
        "01LINTALPHA0000000000001",
        "wiki/alpha.md",
        "Alpha Topic",
        "# Alpha Topic\n",
    ));
    let beta = fixture.add_note(NoteSpec::wiki_entity(
        "01LINTBETA00000000000001",
        "wiki/beta.md",
        "Beta Topic",
        "# Beta Topic\n",
    ));
    let index = fixture.add_note(NoteSpec::wiki_index());
    fixture.add_link(&index, &beta, "Beta Topic");

    let findings = fixture.lint_vault();

    assert_snapshot!(render_findings(&findings), @r###"
OrphanPage|page=01LINTALPHA0000000000001|source=-|severity=Warning|wiki page is orphaned
"###);
}

#[test]
fn stale_claim_reports_newer_source() {
    let fixture = LintFixture::new();
    let source = fixture.add_note(
        NoteSpec::raw_source(
            "01LINTSOURCE000000000001",
            "raw/source.md",
            "Raw Source",
            "# Raw Source\n",
        )
        .updated_at("2026-05-03T00:00:00Z"),
    );
    let page = fixture.add_note(
        NoteSpec::wiki_entity(
            "01LINTSTALE000000000001",
            "wiki/stale.md",
            "Stale Page",
            "# Stale Page\n",
        )
        .updated_at("2026-05-01T00:00:00Z"),
    );
    let index = fixture.add_note(NoteSpec::wiki_index());
    fixture.add_link(&index, &page, "Stale Page");
    fixture.add_source(&page, &source);

    let findings = fixture.lint_vault();

    assert_snapshot!(render_findings(&findings), @r###"
StaleClaim|page=01LINTSTALE000000000001|source=01LINTSOURCE000000000001|severity=Warning|source is newer than wiki page
"###);
}

#[test]
fn missing_cross_reference_ignores_existing_link() {
    let fixture = LintFixture::new();
    let alpha = fixture.add_note(NoteSpec::wiki_entity(
        "01LINTXREFALPHA000000001",
        "wiki/alpha.md",
        "Alpha Topic",
        "# Alpha Topic\n\nMentions Beta Topic and Gamma Topic.\n",
    ));
    let beta = fixture.add_note(NoteSpec::wiki_entity(
        "01LINTXREFBETA0000000001",
        "wiki/beta.md",
        "Beta Topic",
        "# Beta Topic\n",
    ));
    let gamma = fixture.add_note(NoteSpec::wiki_entity(
        "01LINTXREFGAMMA000000001",
        "wiki/gamma.md",
        "Gamma Topic",
        "# Gamma Topic\n",
    ));
    let index = fixture.add_note(NoteSpec::wiki_index());
    fixture.add_link(&index, &alpha, "Alpha Topic");
    fixture.add_link(&index, &beta, "Beta Topic");
    fixture.add_link(&index, &gamma, "Gamma Topic");
    fixture.add_link(&alpha, &gamma, "Gamma Topic");

    let findings = fixture.lint_vault();

    assert_snapshot!(render_findings(&findings), @r###"
MissingCrossReference|page=01LINTXREFALPHA000000001|source=-|severity=Warning|body mentions "Beta Topic" without a note link
"###);
}

#[test]
fn important_concept_counts_raw_sources_and_skips_existing_entity() {
    let fixture = LintFixture::new();
    for (index, path) in ["raw/one.md", "raw/two.md", "raw/three.md"]
        .into_iter()
        .enumerate()
    {
        fixture.add_note(NoteSpec::raw_source(
            &format!("01LINTSOURCE{index:013}"),
            path,
            &format!("Raw Source {index}"),
            "# Source\n\nQuantum Field Theory and Existing Concept appear here.\nQuantum Field Theory repeats.\n",
        ));
    }
    let existing = fixture.add_note(NoteSpec::wiki_entity(
        "01LINTEXISTING0000000001",
        "wiki/existing.md",
        "Existing Concept",
        "# Existing Concept\n",
    ));
    let index = fixture.add_note(NoteSpec::wiki_index());
    fixture.add_link(&index, &existing, "Existing Concept");

    let findings = fixture.lint_vault();

    assert_snapshot!(render_findings(&findings), @r###"
ImportantConceptWithoutPage|page=-|source=-|severity=Warning|concept "Quantum Field Theory" appears in 3 raw sources without a wiki-entity page
"###);
}

#[test]
fn folder_scope_limits_authored_pages() {
    let fixture = LintFixture::new();
    fixture.add_note(NoteSpec::wiki_entity(
        "01LINTSCOPED000000000001",
        "wiki/topic/scoped.md",
        "Scoped Topic",
        "# Scoped Topic\n",
    ));
    fixture.add_note(NoteSpec::wiki_entity(
        "01LINTOUTSIDE00000000001",
        "wiki/other/outside.md",
        "Outside Topic",
        "# Outside Topic\n",
    ));

    let findings = fixture.lint_folder("wiki/topic");

    assert_snapshot!(render_findings(&findings), @r###"
OrphanPage|page=01LINTSCOPED000000000001|source=-|severity=Warning|wiki page is orphaned
"###);
}

struct LintFixture {
    conn: Connection,
    vault: TempDir,
    config: EffectiveConfig,
}

impl LintFixture {
    fn new() -> Self {
        let conn = zk_index::open_memory().expect("local index should open");
        zk_index::ensure_schema(&conn).expect("local index schema should apply");
        let vault = tempfile::tempdir().expect("vault tempdir should be created");
        let config = EffectiveConfig {
            vault_path: vault.path().to_path_buf(),
            local_db_path: vault.path().join(".zk/index.sqlite"),
            wiki: Some(EffectiveWikiConfig {
                root_dir: RelativePath::new("wiki"),
                raw_dir: RelativePath::new("raw"),
                index_path: RelativePath::new("wiki/index.md"),
                log_path: RelativePath::new("wiki/log.md"),
            }),
            ..EffectiveConfig::default()
        };

        Self {
            conn,
            vault,
            config,
        }
    }

    fn add_note(&self, spec: NoteSpec) -> NoteId {
        let NoteSpec {
            id,
            path,
            title,
            note_type,
            body,
            updated_at,
        } = spec;
        let note_id = NoteId::new(&id);
        let relative_path = RelativePath::new(&path);
        let frontmatter = NoteFrontmatter {
            id: note_id.clone(),
            title: title.clone(),
            tags: vec!["wiki".to_string()],
            note_type,
            aliases: Vec::new(),
            created: None,
            updated: None,
            source: None,
            status: None,
            sources: Vec::new(),
            extra: BTreeMap::new(),
        };
        let content = render_note_file(&frontmatter, &body);
        write_text(&self.vault.path().join(relative_path.as_str()), &content);
        let content_hash = ContentHash::from_bytes(content.as_bytes());
        let frontmatter_json =
            serde_json::to_string(&frontmatter).expect("frontmatter should serialize");

        NoteRepo::upsert_note(
            &self.conn,
            UpsertNoteParams {
                note_id: &note_id,
                path: &relative_path,
                title: &title,
                content_hash: &content_hash,
                frontmatter_json: &frontmatter_json,
                created_at: "2026-05-01T00:00:00Z",
                updated_at: &updated_at,
                trashed_at: None,
                is_trashed: false,
            },
        )
        .expect("note should upsert");

        note_id
    }

    fn add_link(&self, source_note_id: &NoteId, target_note_id: &NoteId, raw_target: &str) {
        let mut links = LinkRepo::get_note_links(&self.conn, source_note_id)
            .expect("existing note links should list");
        links.push(ResolvedNoteLink {
            source_note_id: source_note_id.clone(),
            raw_target: raw_target.to_string(),
            target_note_id: Some(target_note_id.clone()),
            link_text: Some(raw_target.to_string()),
            is_ambiguous: false,
        });
        LinkRepo::replace_note_links(&self.conn, source_note_id, &links)
            .expect("note links should replace");
    }

    fn add_source(&self, page_id: &NoteId, source_id: &NoteId) {
        SourcesRepo::replace_for_note(&self.conn, page_id, std::slice::from_ref(source_id))
            .expect("note sources should replace");
    }

    fn lint_vault(&self) -> Vec<WikiLintFinding> {
        LintWikiService::new()
            .lint(
                &self.conn,
                self.vault.path(),
                &self.config,
                &WikiLintScope::Vault,
            )
            .expect("vault lint should succeed")
    }

    fn lint_folder(&self, path: &str) -> Vec<WikiLintFinding> {
        LintWikiService::new()
            .lint(
                &self.conn,
                self.vault.path(),
                &self.config,
                &WikiLintScope::Folder {
                    path: RelativePath::new(path),
                },
            )
            .expect("folder lint should succeed")
    }
}

struct NoteSpec {
    id: String,
    path: String,
    title: String,
    note_type: Option<String>,
    body: String,
    updated_at: String,
}

impl NoteSpec {
    fn wiki_entity(id: &str, path: &str, title: &str, body: &str) -> Self {
        Self::new(id, path, title, Some("wiki-entity"), body)
    }

    fn wiki_index() -> Self {
        Self::new(
            "01LINTINDEX0000000000001",
            "wiki/index.md",
            "Wiki Index",
            Some("wiki-index"),
            "# Wiki Index\n",
        )
    }

    fn raw_source(id: &str, path: &str, title: &str, body: &str) -> Self {
        Self::new(id, path, title, None, body)
    }

    fn updated_at(mut self, updated_at: &str) -> Self {
        self.updated_at = updated_at.to_string();
        self
    }

    fn new(id: &str, path: &str, title: &str, note_type: Option<&str>, body: &str) -> Self {
        Self {
            id: id.to_string(),
            path: path.to_string(),
            title: title.to_string(),
            note_type: note_type.map(str::to_string),
            body: body.to_string(),
            updated_at: "2026-05-01T00:00:00Z".to_string(),
        }
    }
}

fn render_note_file(frontmatter: &NoteFrontmatter, body: &str) -> String {
    let mut content = format!(
        "---\nid: {}\ntitle: {}\ntags:\n  - wiki\n",
        frontmatter.id, frontmatter.title
    );
    if let Some(note_type) = &frontmatter.note_type {
        let _ = writeln!(content, "type: {note_type}");
    }
    content.push_str("---\n\n");
    content.push_str(body);
    content
}

fn render_findings(findings: &[WikiLintFinding]) -> String {
    findings
        .iter()
        .map(|finding| {
            format!(
                "{:?}|page={}|source={}|severity={:?}|{}",
                finding.kind,
                optional_note_id(finding.page_id.as_ref()),
                optional_note_id(finding.source_id.as_ref()),
                finding.severity,
                finding.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn optional_note_id(note_id: Option<&NoteId>) -> &str {
    note_id.map_or("-", NoteId::as_str)
}

fn write_text(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("note file should be written");
}
