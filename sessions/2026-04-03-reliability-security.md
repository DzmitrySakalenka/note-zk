# Session: Reliability and Security

Date: 2026-04-03

## User Concerns

- service crash should not lose data
- full server crash should be recoverable from backup
- backups may be stored off-host such as NAS or S3
- files should be protected from leaks
- API requests and sessions should be secure

## Recommendation

Use layered protection:

- filesystem-first canonical data
- trash for short-term recovery
- regular backups
- off-host encrypted backups
- TLS for remote access
- revocable tokens
- restore-from-canonical-only must remain possible

## Important Architectural Rule

Backups should prioritize canonical files and attachments first, because everything else can be rebuilt more easily.

## User Decisions Added

- built-in backups are preferred
- V1 built-in backup scope should be limited to scheduling, encryption, and retention
- future web app should eventually support the full feature set, not remain read-only
- sync/auth metadata should be included in primary backups

## Session Security Concern

The user explicitly wants protection against:

- stolen login/password
- intercepted requests exposing data or credentials

Recommended answer:

- TLS is mandatory for remote access
- browser sessions should use secure HTTP-only cookies
- sync/editor clients should use revocable device tokens
- passwords should be hashed with Argon2id
- session/token revocation should be first-class
