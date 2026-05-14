// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::{
    fmt::Write as _,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;
use url::Url;
use zk_ai::{
    provider::{StubChatProvider, StubEmbeddingsProvider},
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_app::{RebuildOptions, bootstrap_index_connection, rebuild_index};
use zk_domain::{
    AuthScope, EmbeddingModelId, ExecutionMode, JobId, ProviderId, ProviderSelectorPolicy,
    config::{EffectiveConfig, EffectiveWikiConfig},
    id::{ContentHash, DeviceId, RelativePath},
    note::NoteFrontmatter,
};
use zk_server::{
    ServerDb,
    ai::worker::{AiWorkerDeps, spawn_ai_job_worker},
    config::{AiChatConfig, AiConfig, AiProviderKind, ServerConfig},
    http::build_router,
    repo::{AiJobFilter, AiJobKindFilter, AiJobsRepo, NoteRow, NotesRepo},
    service::{self, IssueTokenSpec},
    state::{ServerState, bootstrap_with_config},
};
use zk_vault::{NormalizationContext, analyze_body, normalize_frontmatter, parse_note_file};

const ORPHAN_ID: &str = "01LINTCLIORPHAN000000001";
const STALE_ID: &str = "01LINTCLISTALE0000000001";
const STALE_SOURCE_ID: &str = "01LINTCLISOURCE000000001";
const ALPHA_ID: &str = "01LINTCLIALPHA0000000001";
const BETA_ID: &str = "01LINTCLIBETA00000000001";
const INDEX_ID: &str = "01LINTCLIINDEX0000000001";
const LOG_ID: &str = "01LINTCLILOG00000000001";

struct CliWikiLintFixture {
    _tempdir: TempDir,
    vault_root: PathBuf,
    home_root: PathBuf,
    db_path: PathBuf,
    server_db_path: PathBuf,
}

impl CliWikiLintFixture {
    fn new() -> Self {
        let tempdir = TempDir::new().expect("tempdir should be created");
        let vault_root = tempdir.path().join("vault");
        let home_root = tempdir.path().join("home");
        let db_path = tempdir.path().join("index.sqlite3");
        let server_db_path = tempdir.path().join("server.sqlite3");
        fs::create_dir(&vault_root).expect("vault dir should be created");
        fs::create_dir(&home_root).expect("home dir should be created");
        write_wiki_config(&vault_root);
        write_lint_notes(&vault_root);

        Self {
            _tempdir: tempdir,
            vault_root,
            home_root,
            db_path,
            server_db_path,
        }
    }

    fn app_config(&self) -> EffectiveConfig {
        EffectiveConfig {
            vault_path: self.vault_root.clone(),
            local_db_path: self.db_path.clone(),
            wiki: Some(EffectiveWikiConfig {
                root_dir: RelativePath::new("wiki"),
                raw_dir: RelativePath::new("raw"),
                index_path: RelativePath::new("wiki/index.md"),
                log_path: RelativePath::new("wiki/log.md"),
            }),
            ..EffectiveConfig::default()
        }
    }

    fn rebuild_local_index(&self) {
        let app_config = self.app_config();
        let mut connection =
            bootstrap_index_connection(&app_config).expect("local index should open");
        rebuild_index(
            &self.vault_root,
            &app_config,
            &mut connection,
            RebuildOptions::default(),
        )
        .expect("local index should rebuild");
    }
}

struct TestServer {
    base_url: Url,
    state: ServerState,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

impl TestServer {
    fn issue_admin_token(&self) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "CLI wiki lint E2E token".to_string(),
                        device_id: None,
                        user_id: None,
                        scopes: vec![AuthScope::Admin],
                    },
                    self.state.started_at(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }

    fn latest_lint_job_id(&self) -> JobId {
        self.state
            .db()
            .with_conn(|connection| {
                let jobs = AiJobsRepo::new(connection).list(AiJobFilter {
                    kind: Some(AiJobKindFilter::LintWiki),
                    limit: Some(1),
                    ..AiJobFilter::default()
                })?;

                Ok(jobs
                    .first()
                    .expect("lint job should have been created")
                    .job_id
                    .clone())
            })
            .expect("lint job should list")
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.join.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wiki_lint_wait_and_show_report_planted_findings() {
    let fixture = CliWikiLintFixture::new();
    fixture.rebuild_local_index();
    let app_config = fixture.app_config();
    let (server, worker_shutdown, worker_handle) = start_server(&fixture, app_config).await;
    let token = server.issue_admin_token();

    let json_assert = wiki_command(&fixture, &server, &token)
        .arg("lint")
        .arg("--wait")
        .arg("--json")
        .assert()
        .success();
    let json_stdout: Value =
        serde_json::from_slice(&json_assert.get_output().stdout).expect("stdout should be JSON");

    assert_eq!(json_stdout, expected_findings_json());
    assert_eq!(
        finding_kind_page_pairs(&json_stdout),
        vec![
            ("orphan_page".to_string(), Some(ORPHAN_ID.to_string())),
            ("stale_claim".to_string(), Some(STALE_ID.to_string())),
            ("missing_xref".to_string(), Some(ALPHA_ID.to_string())),
            ("missing_page".to_string(), None),
        ]
    );

    let job_id = server.latest_lint_job_id();
    let human_assert = wiki_command(&fixture, &server, &token)
        .arg("lint")
        .arg("show")
        .arg(job_id.to_string())
        .assert()
        .success();
    let human_stdout = stdout_text(human_assert.get_output());

    assert_group_order(&human_stdout);
    assert!(human_stdout.contains(ORPHAN_ID));
    assert!(human_stdout.contains("Orphan Topic"));
    assert!(human_stdout.contains(STALE_ID));
    assert!(human_stdout.contains("Stale Topic"));
    assert!(human_stdout.contains(ALPHA_ID));
    assert!(human_stdout.contains("Alpha Topic"));
    assert!(human_stdout.contains("Quantum Field Theory"));

    worker_shutdown.cancel();
    timeout(Duration::from_secs(1), worker_handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");
}

async fn start_server(
    fixture: &CliWikiLintFixture,
    app_config: EffectiveConfig,
) -> (TestServer, CancellationToken, JoinHandle<()>) {
    let state = bootstrap_with_config(test_config(fixture)).expect("server state should bootstrap");
    seed_server_notes(&state, &fixture.vault_root, &app_config);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let local_addr = listener
        .local_addr()
        .expect("test server address should be available");
    let base_url = format!("http://{local_addr}")
        .parse()
        .expect("test server URL should parse");
    let app = build_router(state.clone());
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("test server should serve");
    });
    let worker_shutdown = CancellationToken::new();
    let worker_handle = spawn_ai_job_worker(
        AiWorkerDeps {
            db: Arc::new(
                ServerDb::open(&fixture.server_db_path).expect("server db should open for worker"),
            ),
            selector: selector(),
            config: ai_config(),
            vault_root: fixture.vault_root.clone(),
            app_config,
            worker_device_id: DeviceId::new("cli-wiki-lint-e2e-worker"),
        },
        worker_shutdown.clone(),
    );

    (
        TestServer {
            base_url,
            state,
            shutdown_tx: Some(shutdown_tx),
            join,
        },
        worker_shutdown,
        worker_handle,
    )
}

fn seed_server_notes(state: &ServerState, vault_root: &Path, app_config: &EffectiveConfig) {
    state
        .db()
        .with_conn(|connection| {
            for path in lint_note_paths() {
                NotesRepo::create(
                    connection,
                    &note_row_from_file(state, vault_root, app_config, &path),
                )?;
            }

            Ok(())
        })
        .expect("server notes should seed");
}

fn lint_note_paths() -> Vec<String> {
    let mut paths = vec![
        "wiki/orphan.md".to_string(),
        "raw/stale-source.md".to_string(),
        "wiki/stale.md".to_string(),
        "wiki/alpha.md".to_string(),
        "wiki/beta.md".to_string(),
        "wiki/index.md".to_string(),
        "wiki/log.md".to_string(),
    ];
    for index in 0..3 {
        paths.push(format!("raw/concept-{index}.md"));
    }

    paths
}

fn note_row_from_file(
    state: &ServerState,
    vault_root: &Path,
    config: &EffectiveConfig,
    path: &str,
) -> NoteRow {
    let absolute_path = vault_root.join(path);
    let file_bytes = fs::read(&absolute_path).expect("note bytes should read");
    let (frontmatter, body_markdown) = normalize_note_file(vault_root, config, path);
    let now = state.started_at();

    NoteRow {
        note_id: frontmatter.id.to_string(),
        current_path: path.to_string(),
        title: frontmatter.title.clone(),
        content_hash: ContentHash::from_bytes(&file_bytes).to_string(),
        body_markdown,
        frontmatter_json: serde_json::to_string(&frontmatter)
            .expect("frontmatter JSON should serialize"),
        created_at: now,
        updated_at: now,
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: 1,
    }
}

fn normalize_note_file(
    vault_root: &Path,
    config: &EffectiveConfig,
    path: &str,
) -> (NoteFrontmatter, String) {
    let relative_path = RelativePath::new(path);
    let parsed = parse_note_file(&vault_root.join(path)).expect("note should parse");
    let raw_frontmatter = parsed.frontmatter.expect("note should have frontmatter");
    let body_analysis = analyze_body(&parsed.body_markdown);
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config,
        is_immutable: false,
    };
    let (frontmatter, _) = normalize_frontmatter(raw_frontmatter, &context);

    (frontmatter, parsed.body_markdown)
}

fn wiki_command(fixture: &CliWikiLintFixture, server: &TestServer, token: &str) -> Command {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("--vault")
        .arg(&fixture.vault_root)
        .arg("--db")
        .arg(&fixture.db_path)
        .arg("--server-url")
        .arg(server.base_url.as_str())
        .arg("wiki")
        .env("HOME", &fixture.home_root)
        .env("APPDATA", &fixture.home_root)
        .env("XDG_CONFIG_HOME", fixture.home_root.join("xdg"))
        .env("ZK_DEVICE_TOKEN", token);
    command
}

fn test_config(fixture: &CliWikiLintFixture) -> ServerConfig {
    ServerConfig {
        vault_path: fixture.vault_root.clone(),
        database_path: fixture.server_db_path.clone(),
        listen_addr: "127.0.0.1:0"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        ai: Some(ai_config()),
        ..ServerConfig::default()
    }
}

fn ai_config() -> AiConfig {
    AiConfig {
        provider: AiProviderKind::Stub,
        remote: None,
        local: None,
        stub: None,
        chat: Some(AiChatConfig::default()),
        default_mode: ExecutionMode::Hybrid,
        recompute_concurrency: 1,
        max_attempts: 1,
    }
}

fn selector() -> Arc<ProviderSelector> {
    let mut registry = ProviderRegistry::new();
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(2)));
    registry.register_chat(Arc::new(StubChatProvider::new(model_id(), 1024)));

    Arc::new(ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Hybrid,
            remote_provider: None,
            local_provider: None,
            stub_provider: Some(provider_id()),
        },
    ))
}

fn provider_id() -> ProviderId {
    "stub".parse().expect("provider fixture should parse")
}

fn model_id() -> EmbeddingModelId {
    "test/wiki-lint-cli"
        .parse()
        .expect("model fixture should parse")
}

fn write_lint_notes(vault_root: &Path) {
    write_text(
        &vault_root.join("wiki/orphan.md"),
        &note_file(NoteSpec::wiki_entity(
            ORPHAN_ID,
            "Orphan Topic",
            "# Orphan Topic\n",
        )),
    );
    write_text(
        &vault_root.join("raw/stale-source.md"),
        &note_file(NoteSpec {
            note_id: STALE_SOURCE_ID,
            title: "Stale Source",
            note_type: None,
            sources: None,
            updated: "2026-05-03T00:00:00Z",
            body: "raw facts\n",
        }),
    );
    write_text(
        &vault_root.join("wiki/stale.md"),
        &note_file(NoteSpec {
            note_id: STALE_ID,
            title: "Stale Topic",
            note_type: Some("wiki-entity"),
            sources: Some(&[STALE_SOURCE_ID]),
            updated: "2026-05-01T00:00:00Z",
            body: "# Stale Topic\n",
        }),
    );
    write_text(
        &vault_root.join("wiki/alpha.md"),
        &note_file(NoteSpec::wiki_entity(
            ALPHA_ID,
            "Alpha Topic",
            "# Alpha Topic\n\nMentions Beta Topic without a link.\n",
        )),
    );
    write_text(
        &vault_root.join("wiki/beta.md"),
        &note_file(NoteSpec::wiki_entity(
            BETA_ID,
            "Beta Topic",
            "# Beta Topic\n",
        )),
    );
    for index in 0..3 {
        write_text(
            &vault_root.join(format!("raw/concept-{index}.md")),
            &note_file(NoteSpec {
                note_id: &format!("01LINTCLICONCEPT{index:09}"),
                title: &format!("Concept Source {index}"),
                note_type: None,
                sources: None,
                updated: "2026-05-01T00:00:00Z",
                body: "Quantum Field Theory appears here.\n",
            }),
        );
    }
    write_text(
        &vault_root.join("wiki/index.md"),
        &note_file(NoteSpec {
            note_id: INDEX_ID,
            title: "Wiki Index",
            note_type: Some("wiki-index"),
            sources: None,
            updated: "2026-05-01T00:00:00Z",
            body: "# Wiki Index\n\n[[Stale Topic]]\n[[Alpha Topic]]\n[[Beta Topic]]\n",
        }),
    );
    write_text(
        &vault_root.join("wiki/log.md"),
        &note_file(NoteSpec {
            note_id: LOG_ID,
            title: "Wiki Log",
            note_type: Some("wiki-log"),
            sources: None,
            updated: "2026-05-01T00:00:00Z",
            body: "# Wiki Log\n",
        }),
    );
}

#[derive(Clone, Copy)]
struct NoteSpec<'a> {
    note_id: &'a str,
    title: &'a str,
    note_type: Option<&'a str>,
    sources: Option<&'a [&'a str]>,
    updated: &'a str,
    body: &'a str,
}

impl<'a> NoteSpec<'a> {
    fn wiki_entity(note_id: &'a str, title: &'a str, body: &'a str) -> Self {
        Self {
            note_id,
            title,
            note_type: Some("wiki-entity"),
            sources: None,
            updated: "2026-05-01T00:00:00Z",
            body,
        }
    }
}

fn note_file(spec: NoteSpec<'_>) -> String {
    let NoteSpec {
        note_id,
        title,
        note_type,
        sources,
        updated,
        body,
    } = spec;
    let mut yaml = format!(
        "id: {note_id}\ntitle: {title}\ntags:\n  - wiki\ncreated: 2026-05-01T00:00:00Z\nupdated: {updated}\n"
    );
    if let Some(note_type) = note_type {
        let _ = writeln!(yaml, "type: {note_type}");
    }
    if let Some(sources) = sources {
        let _ = writeln!(yaml, "sources:");
        for source in sources {
            let _ = writeln!(yaml, "  - {source}");
        }
    }

    format!("---\n{yaml}---\n{body}\n")
}

fn write_wiki_config(vault_root: &Path) {
    write_text(
        &vault_root.join(".zk/config.toml"),
        r#"
[wiki]
root_dir = "wiki/"
raw_dir = "raw/"
index_file = "index.md"
log_file = "log.md"
"#,
    );
}

fn write_text(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn assert_group_order(stdout: &str) {
    let orphan = stdout
        .find("orphan_page")
        .expect("orphan group should render");
    let stale = stdout
        .find("stale_claim")
        .expect("stale group should render");
    let missing_xref = stdout
        .find("missing_xref")
        .expect("missing xref group should render");
    let missing_page = stdout
        .find("missing_page")
        .expect("missing page group should render");

    assert!(orphan < stale);
    assert!(stale < missing_xref);
    assert!(missing_xref < missing_page);
}

fn expected_findings_json() -> Value {
    json!({
        "findings": [
            {
                "kind": "orphan_page",
                "page_id": ORPHAN_ID,
                "source_id": null,
                "message": "wiki page is orphaned",
                "severity": "warning"
            },
            {
                "kind": "stale_claim",
                "page_id": STALE_ID,
                "source_id": STALE_SOURCE_ID,
                "message": "source is newer than wiki page",
                "severity": "warning"
            },
            {
                "kind": "missing_xref",
                "page_id": ALPHA_ID,
                "source_id": null,
                "message": "body mentions \"Beta Topic\" without a note link",
                "severity": "warning"
            },
            {
                "kind": "missing_page",
                "page_id": null,
                "source_id": null,
                "message": "concept \"Quantum Field Theory\" appears in 3 raw sources without a wiki-entity page",
                "severity": "warning"
            }
        ]
    })
}

fn finding_kind_page_pairs(output: &Value) -> Vec<(String, Option<String>)> {
    output
        .get("findings")
        .and_then(Value::as_array)
        .expect("findings should be an array")
        .iter()
        .map(|finding| {
            let kind = finding
                .get("kind")
                .and_then(Value::as_str)
                .expect("finding kind should be a string")
                .to_string();
            let page_id = finding
                .get("page_id")
                .and_then(Value::as_str)
                .map(ToString::to_string);

            (kind, page_id)
        })
        .collect()
}

fn stdout_text(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8")
}
