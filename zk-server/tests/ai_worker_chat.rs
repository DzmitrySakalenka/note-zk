// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::{self, BoxStream};
use tokio::time::{advance, timeout};
use tokio_util::sync::CancellationToken;
use zk_ai::{
    AiError, AiResult,
    provider::{
        ChatCapability, ChatChunk, ChatProvider, ChatRequest, StubChatProvider,
        StubEmbeddingsProvider,
    },
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_domain::{
    ConversationId, EmbeddingModelId, ExecutionMode, JobId, MessageRecord, MessageRole, ProviderId,
    ProviderSelectorPolicy, auth::UserId,
};
use zk_server::{
    ServerDb,
    ai::worker::{AiWorkerDeps, spawn_ai_job_worker},
    config::{AiChatConfig, AiConfig, AiProviderKind},
    repo::{
        AiJob, AiJobFilter, AiJobKind, AiJobStatus, AiJobsRepo, ConversationsRepo,
        ListMessagesParams, MessagesRepo, UserRepo,
    },
};

mod common;

use common::unique_test_dir;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn chat_turn_job_persists_messages_and_marks_done() {
    let fixture = WorkerChatFixture::new("ai-worker-chat-success");
    let user_id = user_id("worker-chat-success");
    let conversation_id = fixture.create_conversation(&user_id);
    let job = fixture.enqueue(AiJobKind::ChatTurn {
        conversation_id: conversation_id.clone(),
        request_id: "req-chat-success".to_string(),
        user_query: "What changed in my notes?".to_string(),
    });
    let provider = StubChatProvider::new(chat_model_id(), 1024)
        .with_scripted_response("assistant persisted".to_string());
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(selector_with_chat(Arc::new(provider)), ai_config(1)),
        shutdown.clone(),
    );

    let done = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Done).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    let messages = fixture.messages(&conversation_id);
    assert_eq!(done.attempts, 0);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, "What changed in my notes?");
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content, "assistant persisted");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn chat_turn_provider_error_marks_failed_without_prompt_leak() {
    let fixture = WorkerChatFixture::new("ai-worker-chat-provider-fails");
    let user_id = user_id("worker-chat-provider-fails");
    let conversation_id = fixture.create_conversation(&user_id);
    let user_query = "private prompt content must stay out of job errors";
    let job = fixture.enqueue(AiJobKind::ChatTurn {
        conversation_id,
        request_id: "req-chat-fails".to_string(),
        user_query: user_query.to_string(),
    });
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(
            selector_with_chat(Arc::new(FailingChatProvider)),
            ai_config_with_chat_attempts(1, 1),
        ),
        shutdown.clone(),
    );

    let failed = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Failed).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    let last_error = failed
        .last_error
        .expect("failed job should record an error");
    assert_eq!(failed.attempts, 1);
    assert!(last_error.contains("provider failed safely"));
    assert!(
        !last_error.contains(user_query),
        "last_error leaked user query: {last_error}"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn retried_chat_turn_reuses_user_message_and_finishes_done() {
    let fixture = WorkerChatFixture::new("ai-worker-chat-retry-dedup");
    let user_id = user_id("worker-chat-retry-dedup");
    let conversation_id = fixture.create_conversation(&user_id);
    let job = fixture.enqueue(AiJobKind::ChatTurn {
        conversation_id: conversation_id.clone(),
        request_id: "req-chat-retry-dedup".to_string(),
        user_query: "Retry without duplicating me".to_string(),
    });
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(
            selector_with_chat(Arc::new(FailOnceChatProvider::new())),
            ai_config_with_chat_attempts(2, 2),
        ),
        shutdown.clone(),
    );

    let done = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Done).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    let messages = fixture.messages(&conversation_id);
    assert_eq!(done.attempts, 1);
    assert_eq!(
        messages.last().map(|message| message.role),
        Some(MessageRole::Assistant)
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .count(),
        1
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn chat_turn_uses_chat_specific_retry_budget() {
    let fixture = WorkerChatFixture::new("ai-worker-chat-specific-retry");
    let user_id = user_id("worker-chat-specific-retry");
    let conversation_id = fixture.create_conversation(&user_id);
    let job = fixture.enqueue(AiJobKind::ChatTurn {
        conversation_id,
        request_id: "req-chat-specific-retry".to_string(),
        user_query: "Use chat retry budget".to_string(),
    });
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(
            selector_with_chat(Arc::new(FailOnceChatProvider::new())),
            ai_config_with_chat_attempts(1, 2),
        ),
        shutdown.clone(),
    );

    let done = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Done).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    assert_eq!(done.attempts, 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancelled_chat_turn_job_is_not_claimed() {
    let fixture = WorkerChatFixture::new("ai-worker-chat-cancelled");
    let user_id = user_id("worker-chat-cancelled");
    let conversation_id = fixture.create_conversation(&user_id);
    let job = fixture.enqueue(AiJobKind::ChatTurn {
        conversation_id: conversation_id.clone(),
        request_id: "req-chat-cancelled".to_string(),
        user_query: "Do not process this".to_string(),
    });
    assert!(fixture.cancel(&job.job_id));
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(
            selector_with_chat(Arc::new(FailingChatProvider)),
            ai_config(1),
        ),
        shutdown.clone(),
    );

    advance(Duration::from_secs(2)).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    assert_eq!(fixture.job(&job.job_id).status, AiJobStatus::Failed);
    assert!(fixture.messages(&conversation_id).is_empty());
}

struct WorkerChatFixture {
    db: Arc<ServerDb>,
    vault_root: std::path::PathBuf,
}

impl WorkerChatFixture {
    fn new(prefix: &str) -> Self {
        Self {
            db: Arc::new(ServerDb::open_in_memory().expect("in-memory server db should open")),
            vault_root: unique_test_dir(prefix).join("vault"),
        }
    }

    fn deps(&self, selector: Arc<ProviderSelector>, config: AiConfig) -> AiWorkerDeps {
        AiWorkerDeps {
            db: Arc::clone(&self.db),
            selector,
            config,
            vault_root: self.vault_root.clone(),
        }
    }

    fn create_conversation(&self, user_id: &UserId) -> ConversationId {
        self.db
            .with_conn(|connection| {
                UserRepo::create(
                    connection,
                    user_id.as_str(),
                    user_id.as_str(),
                    "test-password-hash",
                    parse_timestamp("2026-05-05T10:00:00Z"),
                )?;
                ConversationsRepo::create(connection, user_id, Some("Worker chat"))
                    .map(|conversation| conversation.conversation_id)
            })
            .expect("conversation fixture should be created")
    }

    fn enqueue(&self, kind: AiJobKind) -> AiJob {
        self.db
            .with_conn(|connection| {
                AiJobsRepo::new(connection).enqueue(kind, parse_timestamp("2026-05-05T10:01:00Z"))
            })
            .expect("AI job should enqueue")
    }

    fn cancel(&self, job_id: &JobId) -> bool {
        self.db
            .with_conn(|connection| {
                AiJobsRepo::new(connection).cancel(job_id, parse_timestamp("2026-05-05T10:02:00Z"))
            })
            .expect("AI job should cancel")
    }

    fn job(&self, job_id: &JobId) -> AiJob {
        job_by_id(&self.db, job_id)
    }

    fn messages(&self, conversation_id: &ConversationId) -> Vec<MessageRecord> {
        self.db
            .with_conn(|connection| {
                MessagesRepo::list_for_conversation(
                    connection,
                    conversation_id,
                    ListMessagesParams::default(),
                )
            })
            .expect("messages should list")
    }
}

struct FailingChatProvider;

#[async_trait]
impl ChatProvider for FailingChatProvider {
    async fn complete(
        &self,
        _request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        Err(AiError::ProviderRequest(
            "provider failed safely".to_string(),
        ))
    }

    fn capability(&self) -> ChatCapability {
        ChatCapability {
            model: chat_model_id(),
            max_input_tokens: 8192,
            max_output_tokens: 1024,
            supports_streaming: true,
        }
    }
}

struct FailOnceChatProvider {
    calls: AtomicUsize,
}

impl FailOnceChatProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ChatProvider for FailOnceChatProvider {
    async fn complete(
        &self,
        _request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(AiError::ProviderRequest(
                "transient chat provider failure".to_string(),
            ));
        }

        Ok(Box::pin(stream::iter([
            Ok(ChatChunk::Delta {
                text: "assistant after retry".to_string(),
            }),
            Ok(ChatChunk::Done {
                finish_reason: zk_ai::provider::FinishReason::Stop,
                usage: zk_ai::provider::ChatUsage {
                    input_tokens: 10,
                    output_tokens: 3,
                },
            }),
        ])))
    }

    fn capability(&self) -> ChatCapability {
        ChatCapability {
            model: chat_model_id(),
            max_input_tokens: 8192,
            max_output_tokens: 1024,
            supports_streaming: true,
        }
    }
}

async fn wait_for_status(db: &Arc<ServerDb>, job_id: &JobId, status: AiJobStatus) -> AiJob {
    for _ in 0..100 {
        let job = job_by_id(db, job_id);

        if job.status == status {
            return job;
        }

        tokio::task::yield_now().await;
        advance(Duration::from_millis(10)).await;
    }

    panic!("job {job_id} did not reach status {status:?}");
}

fn job_by_id(db: &Arc<ServerDb>, job_id: &JobId) -> AiJob {
    db.with_conn(|connection| {
        AiJobsRepo::new(connection)
            .list(AiJobFilter {
                status: None,
                kind: None,
                limit: None,
            })
            .map(|jobs| {
                jobs.into_iter()
                    .find(|job| job.job_id == *job_id)
                    .expect("job should exist")
            })
    })
    .expect("job list should succeed")
}

fn selector_with_chat(provider: Arc<dyn ChatProvider>) -> Arc<ProviderSelector> {
    let mut registry = ProviderRegistry::new();
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(2)));
    registry.register_chat(provider);

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

fn ai_config(max_attempts: u32) -> AiConfig {
    AiConfig {
        provider: AiProviderKind::Stub,
        remote: None,
        local: None,
        stub: None,
        chat: Some(AiChatConfig::default()),
        default_mode: ExecutionMode::Hybrid,
        recompute_concurrency: 1,
        max_attempts,
    }
}

fn ai_config_with_chat_attempts(max_attempts: u32, chat_max_attempts: u8) -> AiConfig {
    let mut config = ai_config(max_attempts);
    config
        .chat
        .as_mut()
        .expect("chat config fixture should exist")
        .max_attempts = chat_max_attempts;
    config
}

fn provider_id() -> ProviderId {
    "stub".parse().expect("provider fixture should parse")
}

fn chat_model_id() -> EmbeddingModelId {
    "test/chat-model"
        .parse()
        .expect("chat model fixture should parse")
}

fn user_id(value: &str) -> UserId {
    UserId::new(value)
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}
