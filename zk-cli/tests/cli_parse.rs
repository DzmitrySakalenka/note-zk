use clap::Parser;

use zk_cli::{
    cli::{Cli, Command, OutputMode},
    error::{CliError, EXIT_ISSUES, EXIT_OK, EXIT_USAGE, exit_code},
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
        exit_code(&CliError::Internal(anyhow::anyhow!("internal failure"))),
        EXIT_ISSUES
    );
}
