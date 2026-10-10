use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use tuifolio::{store::Store, web};

#[derive(Parser)]
#[command(version, about = "Local-first portfolio tracker with a web interface")]
struct Cli {
    #[arg(long)]
    store: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Web(WebArgs),
}

#[derive(Args)]
struct WebArgs {
    #[arg(long, default_value_t = 3000)]
    port: u16,
    #[arg(long, default_value_t = web::DEFAULT_PRICE_REFRESH_SECONDS, help = "Seconds between automatic price refreshes; 0 disables them")]
    price_refresh_seconds: u64,
}

impl Default for WebArgs {
    fn default() -> Self {
        let options = web::WebOptions::default();
        Self {
            port: options.port,
            price_refresh_seconds: options.price_refresh_interval.as_secs(),
        }
    }
}

impl WebArgs {
    fn options(self) -> web::WebOptions {
        web::WebOptions {
            port: self.port,
            price_refresh_interval: std::time::Duration::from_secs(self.price_refresh_seconds),
        }
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let options = match cli.command {
        Some(Command::Web(args)) => args,
        None => WebArgs::default(),
    };
    web::run(Store::open(cli.store)?, options.options())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_startup_matches_explicit_web_options() {
        let cli = Cli::try_parse_from(["tuifolio"]).unwrap();
        assert!(cli.command.is_none());
        let explicit = Cli::try_parse_from(["tuifolio", "web"]).unwrap();
        let Some(Command::Web(args)) = explicit.command else {
            panic!("expected web command");
        };
        let default = WebArgs::default().options();
        let explicit = args.options();
        assert_eq!(default.port, explicit.port);
        assert_eq!(
            default.price_refresh_interval,
            explicit.price_refresh_interval
        );
    }

    #[test]
    fn explicit_web_options_preserve_custom_store_port_and_disabled_refresh() {
        let cli = Cli::try_parse_from([
            "tuifolio",
            "--store",
            "/tmp/custom-store.json",
            "web",
            "--port",
            "8080",
            "--price-refresh-seconds",
            "0",
        ])
        .unwrap();
        assert_eq!(cli.store, Some(PathBuf::from("/tmp/custom-store.json")));
        let Some(Command::Web(args)) = cli.command else {
            panic!("expected web command");
        };
        let options = args.options();
        assert_eq!(options.port, 8080);
        assert!(options.price_refresh_interval.is_zero());
    }
}
