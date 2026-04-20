use clap::{Parser, error::ErrorKind};
use zk_cli::{cli::Cli, error};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ZK_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let result = match Cli::try_parse() {
        Ok(cli) => zk_cli::run(cli),
        Err(parse_error) => match parse_error.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                parse_error.print().expect("clap help should print");
                Ok(error::EXIT_OK)
            }
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            | ErrorKind::MissingRequiredArgument => {
                parse_error.print().expect("clap help should print");
                Ok(error::EXIT_USAGE)
            }
            _ => Err(error::CliError::Argument(parse_error.to_string())),
        },
    };

    match result {
        Ok(code) => std::process::exit(code),
        Err(cli_error) => {
            eprintln!("{cli_error}");
            std::process::exit(error::exit_code(&cli_error));
        }
    }
}
