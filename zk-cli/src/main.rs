use clap::Parser;
use zk_cli::{
    cli::Cli,
    error::{CliError, EXIT_OK, exit_code},
};

fn main() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_env("ZK_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .init();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if !error.use_stderr() {
                error.print().expect("clap help should print");
                std::process::exit(EXIT_OK);
            }
            let cli_error = CliError::Argument(error.to_string());
            eprintln!("{cli_error}");
            std::process::exit(exit_code(&cli_error));
        }
    };

    match zk_cli::run(cli) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(exit_code(&error));
        }
    }
}
