// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use clap::Parser;

use zk_cli::{
    cli::{
        AdminAiCommand, AdminAiRecomputeScope, AiCommand, AuthCommand, Cli, Command, OutputMode,
        PurgeScope, ResolveStrategy, SyncMode,
    },
    error::{
        CliError, EXIT_AI_DISABLED, EXIT_ISSUES, EXIT_OK, EXIT_PROVIDER_UNAVAILABLE, EXIT_REQUEST,
        EXIT_USAGE, exit_code,
    },
};
use zk_domain::ExecutionMode;

#[test]
fn scan_parses_subcommand_and_shared_flags() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "scan",
        "--vault",
        "vault",
        "--db",
        ".zk/custom.sqlite",
        "--trash-dir",
        "trash",
        "--retention-days",
        "30",
        "--allowed-extensions",
        "md,markdown",
        "--allowed-extensions",
        "txt",
        "--server-url",
        "https://zk.example.com",
        "--device-id",
        "device-cli",
        "--allow-insert-frontmatter=false",
        "--derive-title-from-h1",
        "--json",
        "--check",
    ])
    .expect("scan arguments should parse");

    assert_eq!(
        cli.shared.vault.as_deref(),
        Some(std::path::Path::new("vault"))
    );
    assert_eq!(
        cli.shared.db.as_deref(),
        Some(std::path::Path::new(".zk/custom.sqlite"))
    );
    assert_eq!(cli.shared.trash_dir.as_deref(), Some("trash"));
    assert_eq!(cli.shared.retention_days, Some(30));
    assert_eq!(cli.shared.allow_insert_frontmatter, Some(false));
    assert_eq!(cli.shared.derive_title_from_h1, Some(true));
    assert_eq!(
        cli.shared.allowed_extensions,
        Some(vec![
            "md".to_string(),
            "markdown".to_string(),
            "txt".to_string(),
        ])
    );
    assert_eq!(
        cli.shared.server_url,
        Some(url::Url::parse("https://zk.example.com").expect("URL should parse"))
    );
    assert_eq!(cli.shared.device_id.as_deref(), Some("device-cli"));

    match cli.command {
        Command::Scan(args) => {
            assert!(args.check);
            assert!(!args.write);
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
        other => panic!("expected scan command, got {other:?}"),
    }
}

#[test]
fn optional_bool_flags_parse_bare_flag_as_true() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "status",
        "--allow-insert-frontmatter",
        "--derive-title-from-h1",
    ])
    .expect("bare boolean flags should parse as true");

    assert_eq!(cli.shared.allow_insert_frontmatter, Some(true));
    assert_eq!(cli.shared.derive_title_from_h1, Some(true));
}

#[test]
fn sync_parses_mode_and_json_flags() {
    let cli = Cli::try_parse_from(["zk-cli", "sync", "--pull-only", "--json"])
        .expect("sync arguments should parse");

    match cli.command {
        Command::Sync(args) => {
            assert_eq!(args.mode(), SyncMode::PullOnly);
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
        other => panic!("expected sync command, got {other:?}"),
    }
}

#[test]
fn sync_resolves_each_mode() {
    let cli = Cli::try_parse_from(["zk-cli", "sync"]).expect("sync arguments should parse");
    match cli.command {
        Command::Sync(args) => assert_eq!(args.mode(), SyncMode::Full),
        other => panic!("expected sync command, got {other:?}"),
    }

    let cli = Cli::try_parse_from(["zk-cli", "sync", "--bootstrap"])
        .expect("sync bootstrap arguments should parse");
    match cli.command {
        Command::Sync(args) => assert_eq!(args.mode(), SyncMode::Bootstrap),
        other => panic!("expected sync command, got {other:?}"),
    }

    let cli = Cli::try_parse_from(["zk-cli", "sync", "--push-only"])
        .expect("sync push-only arguments should parse");
    match cli.command {
        Command::Sync(args) => assert_eq!(args.mode(), SyncMode::PushOnly),
        other => panic!("expected sync command, got {other:?}"),
    }

    let cli = Cli::try_parse_from(["zk-cli", "sync", "--pull-only"])
        .expect("sync pull-only arguments should parse");
    match cli.command {
        Command::Sync(args) => assert_eq!(args.mode(), SyncMode::PullOnly),
        other => panic!("expected sync command, got {other:?}"),
    }
}

#[test]
fn sync_rejects_conflicting_modes() {
    let error = Cli::try_parse_from(["zk-cli", "sync", "--push-only", "--pull-only"])
        .expect_err("sync should reject conflicting mode flags");

    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn search_parses_semantic_mode_top_k_and_json_flags() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "search",
        "--semantic",
        "vector search",
        "--top-k",
        "5",
        "--mode",
        "offline",
        "--json",
    ])
    .expect("semantic search arguments should parse");

    match cli.command {
        Command::Search(args) => {
            assert!(args.semantic);
            assert_eq!(args.query, "vector search");
            assert_eq!(args.top_k, 5);
            assert_eq!(args.mode, ExecutionMode::Offline);
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
        other => panic!("expected search command, got {other:?}"),
    }
}

#[test]
fn search_defaults_to_top_ten_and_auto_mode() {
    let cli = Cli::try_parse_from(["zk-cli", "search", "--semantic", "vector search"])
        .expect("semantic search arguments should parse");

    match cli.command {
        Command::Search(args) => {
            assert_eq!(args.top_k, 10);
            assert_eq!(args.mode, ExecutionMode::Hybrid);
        }
        other => panic!("expected search command, got {other:?}"),
    }
}

#[test]
fn search_rejects_top_k_above_one_hundred() {
    let error = Cli::try_parse_from([
        "zk-cli",
        "search",
        "--semantic",
        "vector search",
        "--top-k",
        "101",
    ])
    .expect_err("semantic search should reject too many hits");

    assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
}

#[test]
fn ai_recompute_local_parses_vault_model_dir_and_json() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "ai",
        "recompute-local",
        "--vault",
        "vault",
        "--model-dir",
        "models/minilm",
        "--json",
    ])
    .expect("AI recompute-local arguments should parse");

    assert_eq!(
        cli.shared.vault.as_deref(),
        Some(std::path::Path::new("vault"))
    );
    match cli.command {
        Command::Ai(AiCommand::RecomputeLocal(args)) => {
            assert_eq!(
                args.model_dir.as_deref(),
                Some(std::path::Path::new("models/minilm"))
            );
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
        other => panic!("expected ai recompute-local command, got {other:?}"),
    }
}

#[cfg(not(feature = "ai"))]
#[test]
fn ai_recompute_local_without_ai_feature_returns_usage_error() {
    let cli = Cli::try_parse_from(["zk-cli", "ai", "recompute-local", "--vault", "vault"])
        .expect("AI recompute-local arguments should parse without the feature");

    let error = zk_cli::run(cli).expect_err("AI recompute-local should require the ai feature");

    assert!(
        matches!(error, CliError::Argument(message) if message.contains("recompute-local requires zk-cli built with the ai feature"))
    );
}

#[test]
fn auth_issue_token_parses_command_and_shared_flags() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "auth",
        "issue-token",
        "--server-url",
        "https://zk.example.com",
        "--username",
        "admin",
        "--label",
        "laptop",
        "--scopes",
        "sync,read",
        "--device-id",
        "devA",
    ])
    .expect("auth issue-token arguments should parse");

    assert_eq!(
        cli.shared.server_url,
        Some(url::Url::parse("https://zk.example.com").expect("URL should parse"))
    );
    assert_eq!(cli.shared.device_id.as_deref(), Some("devA"));

    match cli.command {
        Command::Auth(AuthCommand::IssueToken(args)) => {
            assert_eq!(args.username, "admin");
            assert_eq!(args.label, "laptop");
            assert_eq!(args.scopes, "sync,read");
        }
        other => panic!("expected auth issue-token command, got {other:?}"),
    }
}

#[test]
fn auth_issue_token_defaults_scopes_to_sync() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "auth",
        "issue-token",
        "--server-url",
        "https://zk.example.com",
        "--username",
        "admin",
        "--label",
        "laptop",
    ])
    .expect("auth issue-token arguments should parse");

    match cli.command {
        Command::Auth(AuthCommand::IssueToken(args)) => {
            assert_eq!(args.scopes, "sync");
        }
        other => panic!("expected auth issue-token command, got {other:?}"),
    }
}

#[test]
fn admin_ai_recompute_parses_vault_scope() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "admin",
        "ai",
        "recompute",
        "--server-url",
        "https://zk.example.com",
        "--scope",
        "vault",
        "--token-env",
        "ADMIN_TOKEN",
    ])
    .expect("admin ai recompute arguments should parse");

    match cli.command {
        Command::Admin(zk_cli::cli::AdminCommand::Ai(AdminAiCommand::Recompute(args))) => {
            assert_eq!(args.scope, Some(AdminAiRecomputeScope::Vault));
            assert!(args.note.is_none());
            assert_eq!(args.token_env, "ADMIN_TOKEN");
        }
        other => panic!("expected admin ai recompute command, got {other:?}"),
    }
}

#[test]
fn admin_ai_jobs_parses_status_limit_and_json() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "admin",
        "ai",
        "jobs",
        "--server-url",
        "https://zk.example.com",
        "--status",
        "in-flight",
        "--limit",
        "2",
        "--json",
    ])
    .expect("admin ai jobs arguments should parse");

    match cli.command {
        Command::Admin(zk_cli::cli::AdminCommand::Ai(AdminAiCommand::Jobs(args))) => {
            assert_eq!(args.status.as_deref(), Some("in-flight"));
            assert_eq!(args.limit, Some(2));
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
        other => panic!("expected admin ai jobs command, got {other:?}"),
    }
}

#[test]
fn admin_ai_cancel_parses_job_id() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "admin",
        "ai",
        "cancel",
        "01JAR5A3PJEZ3SKG67PN8N6S6S",
        "--server-url",
        "https://zk.example.com",
    ])
    .expect("admin ai cancel arguments should parse");

    match cli.command {
        Command::Admin(zk_cli::cli::AdminCommand::Ai(AdminAiCommand::Cancel(args))) => {
            assert_eq!(args.job_id, "01JAR5A3PJEZ3SKG67PN8N6S6S");
            assert_eq!(args.token_env, "ZK_DEVICE_TOKEN");
        }
        other => panic!("expected admin ai cancel command, got {other:?}"),
    }
}

#[test]
fn admin_ai_rejects_scope_and_note_together() {
    let error = Cli::try_parse_from([
        "zk-cli",
        "admin",
        "ai",
        "recompute",
        "--scope",
        "vault",
        "--note",
        "01JAR5A3PJEZ3SKG67PN8N6S6S",
    ])
    .expect_err("admin ai recompute should reject conflicting scope selectors");

    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn resolve_conflict_parses_keep_local_note_target() {
    let cli = Cli::try_parse_from(["zk-cli", "resolve-conflict", "note-a", "--keep-local"])
        .expect("resolve-conflict arguments should parse");

    match cli.command {
        Command::ResolveConflict(args) => {
            assert_eq!(args.target, "note-a");
            assert!(!args.as_asset);
            assert_eq!(args.strategy(), ResolveStrategy::KeepLocal);
            assert_eq!(OutputMode::from(args.json), OutputMode::Human);
        }
        other => panic!("expected resolve-conflict command, got {other:?}"),
    }
}

#[test]
fn resolve_conflict_parses_keep_remote_asset_json() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "resolve-conflict",
        "asset-a",
        "--as-asset",
        "--keep-remote",
        "--json",
    ])
    .expect("resolve-conflict asset arguments should parse");

    match cli.command {
        Command::ResolveConflict(args) => {
            assert_eq!(args.target, "asset-a");
            assert!(args.as_asset);
            assert_eq!(args.strategy(), ResolveStrategy::KeepRemote);
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
        other => panic!("expected resolve-conflict command, got {other:?}"),
    }
}

#[test]
fn resolve_conflict_requires_resolution_flag() {
    let error = Cli::try_parse_from(["zk-cli", "resolve-conflict", "note-a"])
        .expect_err("resolve-conflict should require one resolution flag");

    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}

#[test]
fn resolve_conflict_rejects_conflicting_resolution_flags() {
    let error = Cli::try_parse_from([
        "zk-cli",
        "resolve-conflict",
        "note-a",
        "--keep-local",
        "--keep-remote",
    ])
    .expect_err("resolve-conflict should reject conflicting resolution flags");

    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn trash_parses_target_and_flags() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "trash",
        "assets/diagram.png",
        "--as-asset",
        "--reason",
        "archive",
        "--dry-run",
        "--json",
    ])
    .expect("trash arguments should parse");

    match cli.command {
        Command::Trash(args) => {
            assert_eq!(args.target, "assets/diagram.png");
            assert!(args.as_asset);
            assert_eq!(args.reason.as_deref(), Some("archive"));
            assert!(args.dry_run);
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
        other => panic!("expected trash command, got {other:?}"),
    }
}

#[test]
fn restore_parses_target_and_alternate_destination() {
    let cli = Cli::try_parse_from([
        "zk-cli",
        "restore",
        "01RESTORETARGET",
        "--to",
        "notes/restored.md",
        "--dry-run",
    ])
    .expect("restore arguments should parse");

    match cli.command {
        Command::Restore(args) => {
            assert_eq!(args.target, "01RESTORETARGET");
            assert_eq!(args.to.as_deref(), Some("notes/restored.md"));
            assert!(args.dry_run);
            assert!(!args.as_asset);
        }
        other => panic!("expected restore command, got {other:?}"),
    }
}

#[test]
fn purge_defaults_to_expired_mode_without_target() {
    let cli = Cli::try_parse_from(["zk-cli", "purge", "--yes", "--json"])
        .expect("purge arguments should parse");

    match cli.command {
        Command::Purge(args) => {
            assert_eq!(args.scope(), PurgeScope::AllExpired);
            assert!(args.yes);
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
        other => panic!("expected purge command, got {other:?}"),
    }
}

#[test]
fn purge_resolves_each_scope() {
    let cli = Cli::try_parse_from(["zk-cli", "purge", "--expired"])
        .expect("purge expired arguments should parse");
    match cli.command {
        Command::Purge(args) => assert_eq!(args.scope(), PurgeScope::AllExpired),
        other => panic!("expected purge command, got {other:?}"),
    }

    let cli = Cli::try_parse_from(["zk-cli", "purge", "01NOTE"])
        .expect("purge note target arguments should parse");
    match cli.command {
        Command::Purge(args) => assert_eq!(
            args.scope(),
            PurgeScope::Target {
                id: "01NOTE".to_string(),
                as_asset: false,
            }
        ),
        other => panic!("expected purge command, got {other:?}"),
    }

    let cli = Cli::try_parse_from(["zk-cli", "purge", "assets/diagram.png", "--as-asset"])
        .expect("purge asset target arguments should parse");
    match cli.command {
        Command::Purge(args) => assert_eq!(
            args.scope(),
            PurgeScope::Target {
                id: "assets/diagram.png".to_string(),
                as_asset: true,
            }
        ),
        other => panic!("expected purge command, got {other:?}"),
    }
}

#[test]
fn purge_rejects_target_and_expired_flag_together() {
    let error = Cli::try_parse_from(["zk-cli", "purge", "--expired", "01TARGET"])
        .expect_err("purge should reject explicit target with --expired");

    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn scan_rejects_conflicting_modes() {
    let error = Cli::try_parse_from(["zk-cli", "scan", "--check", "--write"])
        .expect_err("scan should reject conflicting mode flags");

    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn no_args_returns_usage_error() {
    let error = Cli::try_parse_from(["zk-cli"]).expect_err("cli should require a subcommand");

    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
}

#[test]
fn exit_code_matches_cli_error_policy() {
    assert_eq!(EXIT_OK, 0);
    assert_eq!(EXIT_ISSUES, 1);
    assert_eq!(EXIT_USAGE, 2);
    assert_eq!(EXIT_REQUEST, 2);
    assert_eq!(EXIT_AI_DISABLED, 3);
    assert_eq!(EXIT_PROVIDER_UNAVAILABLE, 4);
    assert_eq!(
        exit_code(&CliError::Argument("bad args".to_string())),
        EXIT_USAGE
    );
    assert_eq!(
        exit_code(&CliError::Bootstrap(anyhow::anyhow!("bootstrap failed"))),
        EXIT_USAGE
    );
    assert_eq!(exit_code(&CliError::Issues), EXIT_ISSUES);
    assert_eq!(
        exit_code(&CliError::Issue("lifecycle issue".to_string())),
        EXIT_ISSUES
    );
    assert_eq!(
        exit_code(&CliError::Request("unauthorized: unauthorized".to_string())),
        EXIT_REQUEST
    );
    assert_eq!(
        exit_code(&CliError::AiDisabled(
            "semantic search is not enabled".to_string()
        )),
        EXIT_AI_DISABLED
    );
    assert_eq!(
        exit_code(&CliError::ProviderUnavailable(
            "local provider unavailable".to_string()
        )),
        EXIT_PROVIDER_UNAVAILABLE
    );
    assert_eq!(
        exit_code(&CliError::Internal(anyhow::anyhow!("internal failure"))),
        EXIT_ISSUES
    );
}
