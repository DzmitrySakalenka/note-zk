# ADR 0015: Chat Protocol

Status: Accepted
Date: 2026-05-07

## Context

TG17 adds server-first chat over retrieved knowledge chunks. ADR 0004
already makes AI server-first by default, with optional local fallback
reserved for offline-capable workflows. ADR 0013 keeps note body
ownership out of the local SQLite index, so chat storage records
conversation artifacts and citation snippets rather than creating a
second local source of truth for note bodies.

Chat spans provider selection, prompt assembly, server persistence,
HTTP streaming, and app offline policy. These protocol choices need one
place to record the V1 behavior and the alternatives TG17 intentionally
left for post-V1 work.

## Decision

### Server-canonical conversations

Conversation, message, and citation storage lives only in server SQLite
in V1. The local app and CLI treat the server as the source of truth for
chat history; a local conversation cache is post-V1. This is the
chat-specific corollary of ADR 0004.

### Streaming by default

`POST /ai/chat` returns Server-Sent Events in V1. Clients that expose a
non-streaming mode collect the SSE stream themselves instead of asking
the server for a separate JSON response.

### Citations as first-class rows

`message_citations` joins assistant messages to `notes` and optional
`note_chunks`, carrying ordinal, similarity, and snippet data. Citations
are not denormalized into the `messages` row. This mirrors relationship
tables such as `note_links` and `semantic_edges`.

### Deterministic prompt assembly

`PromptAssembler` is pure-data and snapshot-tested. Retrieval context,
conversation history, and final prompt text must be deterministic even
though provider output is not.

### Cross-user direct access returns 404

Direct conversation reads for another user return 404 rather than 403.
List endpoints already filter by user, so guessed direct IDs are the
path that could otherwise reveal existence.

### Connected-only V1 chat

Chat and conversation management require server connectivity in V1.
Offline mode, and Hybrid mode when the server probe fails, return
`ProviderUnavailable` instead of queueing work. Local chat fallback is
post-V1.

## Consequences

### Server-canonical conversations

TG17 rejects a local V1 chat cache because no V1 consumer needs offline
replay and conflict rules for local conversation edits do not exist.
Server-side history is immediately available to future remote clients,
but offline chat history browsing waits for a later design.

### Streaming by default

TG17 rejects a server-side non-streaming JSON branch, including a
`stream: false` response path. One SSE code path avoids duplicated
buffering logic and keeps token-by-token delivery available to all
clients.

### Citations as first-class rows

TG17 rejects inline citation JSON on `messages` because note and chunk
joins would become ad hoc and harder to maintain. A table keeps citation
ordering explicit and lets message deletion cascade through the same
relational model used elsewhere in the index.

### Deterministic prompt assembly

TG17 rejects provider-hidden prompt mutation because prompt drift would
be difficult to review when model output is already non-deterministic.
Snapshot tests make prompt changes visible and reviewable.

### Cross-user direct access returns 404

TG17 rejects 403 for foreign conversation IDs because it confirms that a
conversation exists. Returning 404 means clients cannot distinguish a
missing conversation from another user's conversation.

### Connected-only V1 chat

TG17 rejects offline queueing and local generation in V1 because the
local conversation cache and local chat provider are not part of this
milestone. Users get an immediate typed failure instead of hidden pending
state.
