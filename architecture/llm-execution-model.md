# LLM Execution Model

Date: 2026-04-02
Status: Draft

## Question

Where should LLM and embedding workloads run:

- local device
- remote/home server
- both

## Recommendation

Use a policy-based execution model with:

- server-first as the default
- local fallback for offline-capable workflows
- per-capability routing instead of one global rule

This is better than hardcoding "all server-side" or "all client-side."

## Why Not Server-Only

Server-only is attractive because it gives:

- one place to manage models
- one place to manage GPU/CPU resources
- one place to cache embeddings and responses
- consistent behavior across devices

But it has major weaknesses:

- degraded UX during internet/local network outages
- offline devices lose semantic features entirely
- local-first editing becomes second-class

## Why Not Client-Only

Client-only gives:

- best offline behavior
- low latency for local actions on a powerful machine

But it creates problems:

- duplicated model setup across devices
- inconsistent model availability
- weak support for thin clients or future web app
- harder shared indexing and background processing

## Recommended Capability Split

### Chat

Default: server-side

Reason:

- chat benefits from shared retrieval/index state
- easier to manage model/tool execution centrally
- easier to support remote/web clients later
- easier to persist conversations and citations consistently

Offline fallback:

- optional local chat mode when a local model exists and local index is available

### Embeddings

Default: server-side for canonical/shared embeddings

Reason:

- keeps one authoritative embedding/index pipeline
- avoids drift between devices
- easier to manage background jobs

Offline fallback:

- allow local temporary embeddings for local semantic search when offline

Important distinction:

- local offline embeddings can be treated as a cache
- server embeddings remain the shared baseline for synchronized behavior

### Tag extraction / summarization / wiki generation

Default: server-side

Reason:

- background-job friendly
- not always latency-critical
- easier to centralize model config

Offline fallback:

- optional later, not required for MVP

## Key Design Rule

Separate:

- execution location
- data ownership

Even if a model runs locally, the canonical sync/index design should stay consistent.

## Proposed Modes

### Mode 1: Connected

- use server for chat
- use server for canonical embeddings
- sync local files and derived metadata

### Mode 2: Offline local

- user still edits notes normally
- local index continues to work
- keyword search always works
- semantic search works only if local embeddings/model are available
- chat can optionally fall back to local model + local index
- deferred sync and server jobs are queued

### Mode 3: Hybrid catch-up

- device reconnects
- pending file changes sync first
- local temporary derived data may be discarded or reconciled
- server recomputes canonical derived data when needed

## UX Recommendation

The user should see capability state clearly:

- connected: full features
- offline: file editing and keyword search guaranteed
- offline with local AI: semantic search/chat available in degraded local mode

Do not hide degraded state behind silent failures.

## Implementation Guidance

Create provider abstractions for:

- chat completion
- embeddings
- optional reranking/summarization

Then create execution backends:

- local runtime backend
- remote server backend

Clients choose backend by policy and availability, not by special-case feature code.

## MVP Recommendation

Start with:

- server-side chat
- server-side embeddings
- local keyword search always
- local semantic search later as an offline enhancement

This keeps the first implementation simpler while preserving a path to stronger offline support.
