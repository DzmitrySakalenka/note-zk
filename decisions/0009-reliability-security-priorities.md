# ADR 0009: Reliability and Security Prioritize Canonical Data, Encrypted Backups, and Secure Remote Access

Status: Proposed
Date: 2026-04-03

## Decision

The system should prioritize:

- canonical file and attachment recovery first
- encrypted off-host backups
- TLS for remote access
- revocable hashed tokens
- recoverability even if derived indexes are lost
- built-in local encrypted scheduled backups with retention
- secure browser sessions distinct from sync-client device tokens

## Rationale

- canonical data loss is the worst-case failure
- derived data can be rebuilt
- remote access without TLS is unacceptable
- off-host backups are required for real disaster recovery
