// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use clap::Parser;

use zk_cli::{
    cli::{AuthCommand, Cli, Command, OutputMode},
    error::{CliError, EXIT_ISSUES, EXIT_OK, EXIT_REQUEST, EXIT_USAGE, exit_code},
};

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
            assert!(!args.bootstrap);
            assert!(!args.push_only);
            assert!(args.pull_only);
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
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
fn resolve_conflict_parses_keep_local_note_target() {
    let cli = Cli::try_parse_from(["zk-cli", "resolve-conflict", "note-a", "--keep-local"])
        .expect("resolve-conflict arguments should parse");

    match cli.command {
        Command::ResolveConflict(args) => {
            assert_eq!(args.target, "note-a");
            assert!(!args.as_asset);
            assert!(args.keep_local);
            assert!(!args.keep_remote);
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
            assert!(!args.keep_local);
            assert!(args.keep_remote);
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
            assert!(
                !args.expired,
                "default expired mode should not require the flag"
            );
            assert!(args.target.is_none());
            assert!(args.yes);
            assert_eq!(OutputMode::from(args.json), OutputMode::Json);
        }
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
        exit_code(&CliError::Internal(anyhow::anyhow!("internal failure"))),
        EXIT_ISSUES
    );
}
