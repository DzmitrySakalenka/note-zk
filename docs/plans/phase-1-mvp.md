# Phase 1 MVP

Date: 2026-04-03

## Objective

Deliver a local-only Rust CLI application that can scan and normalize a markdown vault.

## Features

- scan vault recursively
- parse markdown notes with YAML frontmatter
- require `title` and `tags`
- auto-generate missing `id`
- detect and index assets
- extract note links and external links
- build local SQLite index
- show status and validation output via CLI

## Success Criteria

- works on a real personal note vault
- does not corrupt notes during normalization
- creates a trustworthy local index
- provides enough visibility to debug data issues
