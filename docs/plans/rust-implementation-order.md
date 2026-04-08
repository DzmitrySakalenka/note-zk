# Rust Implementation Order

Date: 2026-04-03

## Recommended Order

1. `zk-domain`
2. `zk-vault`
3. `zk-index`
4. `zk-app`
5. `zk-cli`
6. `zk-sync`
7. `zk-server`
8. `zk-auth`
9. `zk-ai`
10. `zk-backup`
11. `zk-worker`

## Why

This order gives:

- local-first functionality early
- fast feedback from CLI before network/API complexity
- sync before AI
- operational features after core reliability exists
