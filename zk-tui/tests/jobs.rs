#![allow(unused_crate_dependencies)]

use std::{
    fs, io,
    sync::mpsc::{Receiver, RecvTimeoutError, sync_channel},
    thread,
    time::{Duration, Instant},
};

use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::Rect,
    widgets::{Paragraph, Widget},
};
use zeroize::Zeroizing;
#[cfg(feature = "ai")]
use zk_app::{ChatRequest, SemanticSearchRequest};
use zk_domain::id::RelativePath;
#[cfg(feature = "ai")]
use zk_domain::{ConversationId, ExecutionMode};
use zk_tui::{
    app::{App, Screen, ScreenAction, ScreenCtx},
    event::{
        AppEvent, AppEventSender, BackgroundResult, EventLoop, SystemClock, TerminalEventSource,
    },
    jobs::{
        ConflictItemKind, ConflictResolutionStrategy, ConflictResolutionTarget,
        IssueDeviceTokenIntent, JobContext, JobHandle, JobKind, JobResult, JobRunner, JobSpec,
        MockJob, NoteFilter, NoteListRequest, ResolveConflictIntent, ScanArgs, SyncIntent,
    },
};

const WIDTH: u16 = 64;
const HEIGHT: u16 = 6;
const EVENT_CHANNEL_CAPACITY: usize = 64;

struct StubScreen;

impl Screen for StubScreen {
    fn title(&self) -> &'static str {
        "Jobs Stub"
    }

    fn handle_event(&mut self, _event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        ScreenAction::Stay
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        Paragraph::new("jobs body").render(area, frame.buffer_mut());
    }
}

struct TimeoutInput;

impl TerminalEventSource for TimeoutInput {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        if !timeout.is_zero() {
            thread::sleep(timeout);
        }

        Ok(false)
    }

    fn read(&mut self) -> io::Result<crossterm::event::Event> {
        Err(io::Error::other("timeout input has no terminal events"))
    }
}

#[test]
fn job_runner_public_api_is_reachable() {
    fn assert_start_signature(_start: fn(AppEventSender, JobContext) -> JobRunner) {}
    fn assert_enqueue_signature(_enqueue: fn(&JobRunner, JobSpec) -> JobHandle) {}
    fn assert_cancel_signature(_cancel: fn(&JobHandle)) {}

    assert_start_signature(JobRunner::start);
    assert_enqueue_signature(JobRunner::enqueue);
    assert_cancel_signature(JobHandle::cancel);
    assert_eq!(JobSpec::StatusSummary.kind(), JobKind::StatusSummary);
    assert_eq!(
        JobSpec::ListNotes(NoteListRequest {
            limit: 200,
            offset: 0,
            filter: NoteFilter::All,
        })
        .kind(),
        JobKind::ListNotes
    );
    assert_eq!(
        JobSpec::ReadNoteBody {
            path: RelativePath::new("notes/a.md"),
        }
        .kind(),
        JobKind::ReadNoteBody
    );
    assert_eq!(
        JobSpec::ScanVault(ScanArgs {
            include_trashed: true,
        })
        .kind(),
        JobKind::ScanVault
    );
    assert_eq!(JobSpec::RebuildIndex.kind(), JobKind::RebuildIndex);
    assert_eq!(JobSpec::ListConflicts.kind(), JobKind::ListConflicts);
    assert_eq!(
        JobSpec::ResolveConflict(ResolveConflictIntent {
            target: ConflictResolutionTarget {
                kind: ConflictItemKind::Note,
                item_id: "01TUI15NOTE000000000001".to_string(),
                path: "notes/conflict.md".to_string(),
            },
            strategy: ConflictResolutionStrategy::KeepLocal,
        })
        .kind(),
        JobKind::ResolveConflict
    );
    assert_eq!(
        JobSpec::IssueDeviceToken(IssueDeviceTokenIntent {
            username: "admin".to_string(),
            label: "laptop".to_string(),
            scopes: vec![zk_domain::AuthScope::Sync],
            password: Zeroizing::new("secret".to_string()),
        })
        .kind(),
        JobKind::IssueDeviceToken
    );
    assert_eq!(JobSpec::AdminBackupList.kind(), JobKind::AdminBackupList);
    assert_eq!(
        JobSpec::AdminBackupManifest {
            backup_id: "01HZZZZZZZZZZZZZZZZZZZZZZZ".to_string(),
        }
        .kind(),
        JobKind::AdminBackupManifest
    );
    assert_eq!(
        JobSpec::AdminBackupTrigger.kind(),
        JobKind::AdminBackupTrigger
    );
    assert_eq!(
        JobSpec::AdminAiJobsList {
            status: None,
            limit: None,
        }
        .kind(),
        JobKind::AdminAiJobsList
    );
    assert_eq!(
        JobSpec::AdminAiCancel {
            job_id: "01JAR5A3PJEZ3SKG67PN8N6S6S".to_string(),
        }
        .kind(),
        JobKind::AdminAiCancel
    );
    assert_eq!(
        JobSpec::AdminAiRecomputeVault.kind(),
        JobKind::AdminAiRecomputeVault
    );
    for intent in [
        SyncIntent::DeviceStatus,
        SyncIntent::Bootstrap,
        SyncIntent::Push,
        SyncIntent::Pull,
        SyncIntent::FullCycle,
    ] {
        assert_eq!(JobSpec::Sync(intent).kind(), JobKind::Sync);
    }
}

#[cfg(feature = "ai")]
#[test]
fn semantic_search_job_kind_is_reachable() {
    assert_eq!(
        JobSpec::SemanticSearch {
            request: SemanticSearchRequest {
                query: "graph memory".to_string(),
                top_k: 10,
                mode_override: Some(ExecutionMode::Hybrid),
            },
        }
        .kind(),
        JobKind::SemanticSearch
    );
}

#[cfg(feature = "ai")]
#[test]
fn chat_job_kinds_are_reachable() {
    let conversation_id = ConversationId::new("01JAR5A3PJEZ3SKG67PN8N6S6S");

    assert_eq!(
        JobSpec::ChatListConversations {
            include_archived: false,
            mode: ExecutionMode::Hybrid,
        }
        .kind(),
        JobKind::ChatListConversations
    );
    assert_eq!(
        JobSpec::ChatGetConversation {
            conversation_id: conversation_id.clone(),
            mode: ExecutionMode::Hybrid,
        }
        .kind(),
        JobKind::ChatGetConversation
    );
    assert_eq!(
        JobSpec::ChatStream {
            request: chat_request(conversation_id),
            mode: ExecutionMode::Connected,
        }
        .kind(),
        JobKind::ChatStream
    );
}

#[cfg(feature = "ai")]
#[tokio::test(flavor = "current_thread")]
async fn runner_streams_chat_chunks_from_mock_server() {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    const TOKEN_ENV: &str = "CARGO_MANIFEST_DIR";
    const CONVERSATION_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
    const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6N";
    const CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6C";

    let expected_token = std::env::var(TOKEN_ENV).expect("cargo should set manifest dir");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ai/chat"))
        .and(header("authorization", format!("Bearer {expected_token}")))
        .and(body_json(json!({
            "conversation_id": CONVERSATION_ID,
            "user_query": "hello",
            "top_k": 8,
            "stream": true,
        })))
        .respond_with(ResponseTemplate::new(200).append_header("content-type", "text/event-stream").set_body_string(format!(
            "event: delta\ndata: {{\"text\":\"hello \"}}\n\n\
             event: delta\ndata: {{\"text\":\"from \"}}\n\n\
             event: delta\ndata: {{\"text\":\"stub\"}}\n\n\
             event: citation\ndata: {{\"note_id\":\"{NOTE_ID}\",\"chunk_id\":\"{CHUNK_ID}\",\"similarity\":0.91,\"snippet\":\"source text\",\"ordinal\":1}}\n\n\
             event: done\ndata: {{\"finish_reason\":\"stop\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}\n\n"
        )))
        .expect(1)
        .mount(&server)
        .await;

    let (tx, rx) = sync_channel(EVENT_CHANNEL_CAPACITY);
    let mut config = zk_app::EffectiveConfig {
        vault_path: std::path::PathBuf::from("/vault"),
        ..zk_app::EffectiveConfig::default()
    };
    config.sync.server_url = Some(server.uri().parse().expect("mock server URL should parse"));
    config.sync.bearer_token_env = TOKEN_ENV.to_string();
    let runner = JobRunner::start(
        tx,
        JobContext::new(config, std::path::PathBuf::from("/vault")),
    );

    runner.enqueue(JobSpec::ChatStream {
        request: chat_request(ConversationId::new(CONVERSATION_ID)),
        mode: ExecutionMode::Connected,
    });

    let mut deltas = Vec::new();
    let mut citations = Vec::new();
    let mut done = None;
    while done.is_none() {
        match recv_background_payload(&rx, Duration::from_secs(1)) {
            ChatPayload::Delta(delta) => deltas.push(delta.text),
            ChatPayload::Citation(citation) => citations.push(citation.citation),
            ChatPayload::Done(stream_done) => done = Some(stream_done),
        }
    }

    assert_eq!(deltas, ["hello ", "from ", "stub"]);
    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].note_id.as_str(), NOTE_ID);
    assert_eq!(
        done.expect("stream should finish").conversation_id.as_str(),
        CONVERSATION_ID
    );
    server.verify().await;
}

#[test]
fn runner_reads_note_body_preview_from_disk() {
    let temp = tempfile::tempdir().expect("temp dir should create");
    let notes_dir = temp.path().join("notes");
    fs::create_dir_all(&notes_dir).expect("notes dir should create");
    let body = (1..=205)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        notes_dir.join("long.md"),
        format!("---\nid: 01HZ3KQBFG9T2R5XJ4N8VW6YPA\ntitle: Long\n---\n{body}\n"),
    )
    .expect("note should write");

    let (tx, rx) = sync_channel(EVENT_CHANNEL_CAPACITY);
    let config = zk_app::EffectiveConfig {
        vault_path: temp.path().to_path_buf(),
        ..zk_app::EffectiveConfig::default()
    };
    let runner = JobRunner::start(tx, JobContext::new(config, temp.path().to_path_buf()));

    runner.enqueue(JobSpec::ReadNoteBody {
        path: RelativePath::new("notes/long.md"),
    });

    let JobResult::NoteBody(result) = recv_job_result(&rx, Duration::from_secs(1)) else {
        panic!("expected note body result");
    };

    assert_eq!(result.path.as_str(), "notes/long.md");
    assert_eq!(result.preview.lines().count(), 200);
    assert!(result.preview.contains("line 200"));
    assert!(!result.preview.contains("line 201"));
}

#[test]
fn runner_executes_mock_jobs_fifo() {
    let (tx, rx) = sync_channel(EVENT_CHANNEL_CAPACITY);
    let runner = JobRunner::start(tx, test_context());

    runner.enqueue(JobSpec::Mock(MockJob::new(
        "first",
        Duration::from_millis(120),
    )));
    runner.enqueue(JobSpec::Mock(MockJob::new("second", Duration::ZERO)));

    let first = recv_job_result(&rx, Duration::from_secs(1));
    let second = recv_job_result(&rx, Duration::from_secs(1));

    assert_mock_result(first, "first");
    assert_mock_result(second, "second");
}

#[test]
fn cancelled_mock_job_returns_cancelled_result() {
    let (tx, rx) = sync_channel(EVENT_CHANNEL_CAPACITY);
    let runner = JobRunner::start(tx, test_context());
    let handle = runner.enqueue(JobSpec::Mock(MockJob::new(
        "slow",
        Duration::from_millis(500),
    )));

    handle.cancel();

    let result = recv_job_result(&rx, Duration::from_millis(250));
    assert!(handle.is_cancelled());
    assert!(matches!(
        result,
        JobResult::Cancelled {
            kind: JobKind::Mock,
        }
    ));
}

#[test]
fn render_loop_receives_ticks_while_mock_job_is_in_flight() {
    let event_loop =
        EventLoop::start_with_source(TimeoutInput, SystemClock, Duration::from_millis(50));
    let runner = JobRunner::start(event_loop.sender(), test_context());
    let started = Instant::now();

    runner.enqueue(JobSpec::Mock(MockJob::new(
        "slow",
        Duration::from_millis(500),
    )));

    let mut tick_count = 0;
    loop {
        match event_loop
            .recv()
            .expect("event loop should remain connected")
        {
            AppEvent::Tick => tick_count += 1,
            AppEvent::Background(result) => {
                let result = downcast_job_result(result);
                assert_mock_result(result, "slow");
                break;
            }
            AppEvent::Key(_) | AppEvent::Resize(_, _) => {}
        }
    }

    assert!(started.elapsed() >= Duration::from_millis(500));
    assert!(
        tick_count >= 5,
        "expected at least 5 ticks while job ran, got {tick_count}"
    );
}

#[test]
fn app_renders_error_toast_for_background_job_errors() {
    let ctx = ScreenCtx::for_test().expect("screen context should initialize");
    let mut app = App::with_screens(vec![Box::new(StubScreen)], ctx);
    let mut terminal = test_terminal();

    assert!(
        app.handle_event(AppEvent::Background(BackgroundResult::new(Box::new(
            JobResult::Error {
                kind: JobKind::StatusSummary,
                error: anyhow::anyhow!("status failed"),
            },
        ))))
    );
    draw(&mut terminal, &mut app);

    let output = rendered_rows(&terminal);
    assert!(output.contains("error: status failed"));
}

fn test_context() -> JobContext {
    let config = zk_app::EffectiveConfig::default();
    JobContext::new(config.clone(), config.vault_path.clone())
}

fn recv_job_result(rx: &Receiver<AppEvent>, timeout: Duration) -> JobResult {
    let event = rx
        .recv_timeout(timeout)
        .unwrap_or_else(|error| match error {
            RecvTimeoutError::Timeout => panic!("timed out waiting for job result"),
            RecvTimeoutError::Disconnected => panic!("job event channel disconnected"),
        });

    let AppEvent::Background(result) = event else {
        panic!("expected background job result");
    };

    downcast_job_result(result)
}

#[cfg(feature = "ai")]
fn recv_background_payload(rx: &Receiver<AppEvent>, timeout: Duration) -> ChatPayload {
    let event = rx
        .recv_timeout(timeout)
        .unwrap_or_else(|error| match error {
            RecvTimeoutError::Timeout => panic!("timed out waiting for chat payload"),
            RecvTimeoutError::Disconnected => panic!("job event channel disconnected"),
        });

    let AppEvent::Background(result) = event else {
        panic!("expected background chat payload");
    };
    let payload = result.into_payload();
    if payload.is::<zk_tui::jobs::ChatStreamDelta>() {
        return ChatPayload::Delta(
            *payload
                .downcast::<zk_tui::jobs::ChatStreamDelta>()
                .expect("chat delta should downcast"),
        );
    }
    if payload.is::<zk_tui::jobs::ChatStreamCitation>() {
        return ChatPayload::Citation(
            *payload
                .downcast::<zk_tui::jobs::ChatStreamCitation>()
                .expect("chat citation should downcast"),
        );
    }

    let job_result = *payload
        .downcast::<JobResult>()
        .expect("chat terminal payload should downcast to JobResult");
    let JobResult::ChatStreamDone(done) = job_result else {
        panic!("expected chat stream done result");
    };
    ChatPayload::Done(done)
}

#[cfg(feature = "ai")]
enum ChatPayload {
    Delta(zk_tui::jobs::ChatStreamDelta),
    Citation(zk_tui::jobs::ChatStreamCitation),
    Done(zk_tui::jobs::ChatStreamDone),
}

fn downcast_job_result(result: BackgroundResult) -> JobResult {
    let Ok(result) = result.into_payload().downcast::<JobResult>() else {
        panic!("background payload should downcast to JobResult");
    };

    *result
}

fn assert_mock_result(result: JobResult, expected_label: &str) {
    let JobResult::Mock(mock) = result else {
        panic!("expected mock job result");
    };

    assert_eq!(mock.label, expected_label);
}

fn test_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize")
}

fn draw(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    terminal
        .draw(|frame| app.render(frame))
        .expect("app should render to test backend");
}

fn rendered_rows(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(WIDTH as usize)
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(feature = "ai")]
fn chat_request(conversation_id: ConversationId) -> ChatRequest {
    ChatRequest {
        conversation_id,
        user_query: "hello".to_string(),
        top_k: 8,
        stream: true,
    }
}
