use std::collections::VecDeque;

use futures::{
    StreamExt,
    stream::{self, BoxStream},
};
use serde::Deserialize;
use zk_ai::provider::{ChatChunk, ChatUsage, FinishReason};
use zk_domain::Citation;
use zk_sync::{SyncError, SyncResult};

use crate::{AppError, Result as AppResult};

#[derive(Debug, Deserialize)]
struct DeltaPayload {
    text: String,
}

#[derive(Debug, Deserialize)]
struct DonePayload {
    finish_reason: FinishReason,
    usage: ChatUsage,
}

#[derive(Debug, Deserialize)]
struct ErrorPayload {
    code: String,
    message: String,
}

struct ChatSseState {
    body: BoxStream<'static, SyncResult<Vec<u8>>>,
    buffer: Vec<u8>,
    pending: VecDeque<AppResult<ChatChunk>>,
    saw_done: bool,
    failed: bool,
}

impl ChatSseState {
    fn new(body: BoxStream<'static, SyncResult<Vec<u8>>>) -> Self {
        Self {
            body,
            buffer: Vec::new(),
            pending: VecDeque::new(),
            saw_done: false,
            failed: false,
        }
    }
}

pub(super) fn chat_sse_stream(
    body: BoxStream<'static, SyncResult<Vec<u8>>>,
) -> BoxStream<'static, AppResult<ChatChunk>> {
    Box::pin(stream::unfold(
        ChatSseState::new(body),
        |mut state| async move {
            loop {
                if let Some(chunk) = state.pending.pop_front() {
                    return Some((chunk, state));
                }
                if state.failed || state.saw_done {
                    return None;
                }

                match state.body.next().await {
                    Some(Ok(bytes)) => read_bytes(&mut state, bytes),
                    Some(Err(error)) => push_failure(&mut state, sync_to_network_error(&error)),
                    None => finish_eof(&mut state),
                }
            }
        },
    ))
}

fn read_bytes(state: &mut ChatSseState, bytes: Vec<u8>) {
    state.buffer.extend(bytes);
    drain_complete_frames(state);
}

fn drain_complete_frames(state: &mut ChatSseState) {
    while let Some(index) = find_frame_boundary(&state.buffer) {
        let frame = state.buffer[..index].to_vec();
        state.buffer.drain(..index + 2);
        process_frame_bytes(state, &frame);

        if state.failed || state.saw_done {
            return;
        }
    }
}

fn finish_eof(state: &mut ChatSseState) {
    if !state.buffer.iter().all(u8::is_ascii_whitespace) {
        let frame = std::mem::take(&mut state.buffer);
        process_frame_bytes(state, &frame);
    }

    if !state.failed && !state.saw_done {
        push_failure(
            state,
            invalid_sse_error("chat stream ended before done".to_string()),
        );
    }
}

fn find_frame_boundary(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|window| window == b"\n\n")
}

fn process_frame_bytes(state: &mut ChatSseState, frame: &[u8]) {
    let frame = match std::str::from_utf8(frame) {
        Ok(frame) => frame,
        Err(error) => {
            push_failure(state, invalid_sse_error(error.to_string()));
            return;
        }
    };

    process_frame(state, frame);
}

fn process_frame(state: &mut ChatSseState, frame: &str) {
    let Some((event_name, data)) = parse_frame(frame) else {
        return;
    };

    match event_name.as_str() {
        "delta" => push_json_chunk::<DeltaPayload>(state, &data, |payload| ChatChunk::Delta {
            text: payload.text,
        }),
        "citation" => push_json_chunk::<Citation>(state, &data, ChatChunk::Citation),
        "done" => {
            push_json_chunk::<DonePayload>(state, &data, |payload| ChatChunk::Done {
                finish_reason: payload.finish_reason,
                usage: payload.usage,
            });
            if !state.failed {
                state.saw_done = true;
            }
        }
        "error" => match serde_json::from_str::<ErrorPayload>(&data) {
            Ok(payload) => push_failure(
                state,
                AppError::ApiError {
                    status: 0,
                    code: payload.code,
                    message: payload.message,
                },
            ),
            Err(error) => push_failure(state, invalid_sse_error(error.to_string())),
        },
        event => push_failure(
            state,
            invalid_sse_error(format!("unsupported SSE event: {event}")),
        ),
    }
}

fn parse_frame(frame: &str) -> Option<(String, String)> {
    let mut event_name = None;
    let mut data_lines = Vec::new();

    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_string());
        }
    }

    event_name.map(|name| (name, data_lines.join("\n")))
}

fn push_json_chunk<T>(state: &mut ChatSseState, data: &str, build: impl FnOnce(T) -> ChatChunk)
where
    T: serde::de::DeserializeOwned,
{
    match serde_json::from_str::<T>(data) {
        Ok(payload) => state.pending.push_back(Ok(build(payload))),
        Err(error) => push_failure(state, invalid_sse_error(error.to_string())),
    }
}

fn push_failure(state: &mut ChatSseState, error: AppError) {
    state.pending.push_back(Err(error));
    state.failed = true;
}

fn invalid_sse_error(message: String) -> AppError {
    AppError::ApiError {
        status: 0,
        code: "invalid_sse".to_string(),
        message,
    }
}

fn sync_to_network_error(error: &SyncError) -> AppError {
    AppError::Network {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, stream};
    use zk_ai::provider::ChatChunk;

    use super::chat_sse_stream;

    #[test]
    fn split_utf8_frame_decodes_after_boundary() {
        pollster::block_on(async {
            let frame = "event: delta\ndata: {\"text\":\"hé\"}\n\n\
                         event: done\ndata: {\"finish_reason\":\"stop\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n\n";
            let split_at = frame
                .as_bytes()
                .iter()
                .position(|byte| *byte == 0xc3)
                .expect("fixture should contain a multibyte character")
                + 1;
            let chunks = vec![
                Ok(frame.as_bytes()[..split_at].to_vec()),
                Ok(frame.as_bytes()[split_at..].to_vec()),
            ];

            let parsed = chat_sse_stream(Box::pin(stream::iter(chunks)))
                .collect::<Vec<_>>()
                .await;

            assert!(matches!(&parsed[0], Ok(ChatChunk::Delta { text }) if text == "hé"));
            assert!(matches!(&parsed[1], Ok(ChatChunk::Done { .. })));
        });
    }
}
