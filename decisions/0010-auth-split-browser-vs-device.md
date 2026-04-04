# ADR 0010: Separate Browser Sessions From Device Tokens

Status: Proposed
Date: 2026-04-03

## Decision

Use different authentication models for:

- browser/web UI
- sync/editor/mobile clients

## Browser/Web UI

Use:

- password-based login or equivalent primary auth
- short-lived session
- secure HTTP-only cookies
- session rotation / refresh

## Device Clients

Use:

- long-lived revocable device tokens
- one token per device/client installation where practical

## Rationale

Browser sessions and sync clients have different threat models and storage models.

Trying to use one shared long-lived token style for both is weaker and less flexible.

## Security Requirements

- TLS for all remote access
- Argon2id password hashing
- token hashing at rest
- session/token revoke capability
