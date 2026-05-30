use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rust_decimal::Decimal;

use tuifolio::accounting::build_report;
use tuifolio::importer::import_delta_dir;
use tuifolio::price_sync::{add_manual_price, sync_free_crypto_prices};
use tuifolio::store::Store;
use tuifolio::tui;

#[derive(Parser)]
#[command(version, about = "Local-first Delta-style portfolio TUI")]
struct Cli {
    #[arg(long)]
    store: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Import {
        #[arg(default_value = "delta-exports")]
        dir: PathBuf,
    },
    Rebuild {
        #[arg(default_value = "delta-exports")]
        dir: PathBuf,
    },
    Tui,
    Summary,
    Holding {
        symbol: String,
    },
    MissingPrices,
    SyncPrices,
    AddPrice {
        symbol: String,
        price: Decimal,
        currency: String,
    },
    Base {
        currency: String,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let mut store = Store::open(cli.store)?;
    match cli.command.unwrap_or(Command::Tui) {
        Command::Import { dir } => import_command(&mut store, dir)?,
        Command::Rebuild { dir } => rebuild_command(&mut store, dir)?,
        Command::Tui => tui_command(&mut store)?,
        Command::Summary => summary_command(&store),
        Command::Holding { symbol } => holding_command(&store, &symbol),
        Command::MissingPrices => missing_prices_command(&store),
        Command::SyncPrices => sync_prices_command(&mut store)?,
        Command::AddPrice {
            symbol,
            price,
            currency,
        } => add_price_command(&mut store, &symbol, price, &currency)?,
        Command::Base { currency } => base_command(&mut store, currency)?,
    }
    Ok(())
}

fn import_command(store: &mut Store, dir: PathBuf) -> anyhow::Result<()> {
    let summary = import_delta_dir(store, &dir)?;
    store.save()?;
    println!("imported {}, skipped {}", summary.imported, summary.skipped);
    println!("store: {}", store.path().display());
    Ok(())
}

fn rebuild_command(store: &mut Store, dir: PathBuf) -> anyhow::Result<()> {
    store.reset();
    import_command(store, dir)
}

fn tui_command(store: &mut Store) -> anyhow::Result<()> {
    tui::run(&mut store.data)?;
    store.save()
}

fn summary_command(store: &Store) {
    let report = build_report(&store.data);
    println!("Base: {}", report.base_currency);
    println!("Net worth: {}", report.total_value.round_dp(2));
    println!(
        "Unrealized PnL: {}",
        report.total_unrealized_pnl.round_dp(2)
    );
    println!("Holdings: {}", report.holdings.len());
    println!("Negative balances: {}", report.negative_balances.len());
    for portfolio in report.portfolios {
        println!(
            "{}: {} missing={}",
            portfolio.name,
            portfolio.value.round_dp(2),
            portfolio.unresolved
        );
    }
}

fn missing_prices_command(store: &Store) {
    let report = build_report(&store.data);
    for holding in report.holdings.iter().filter(|holding| holding.stale_price) {
        println!(
            "{} {} ({:?})",
            holding.portfolio, holding.symbol, holding.kind
        );
    }
}

fn holding_command(store: &Store, symbol: &str) {
    let report = build_report(&store.data);
    for holding in report
        .holdings
        .iter()
        .filter(|holding| holding.symbol.eq_ignore_ascii_case(symbol))
    {
        println!(
            "{} {} quantity={} value={} {}",
            holding.portfolio,
            holding.symbol,
            holding.quantity,
            holding
                .value
                .map(|value| value.round_dp(2).to_string())
                .unwrap_or_else(|| "n/a".into()),
            report.base_currency
        );
    }
}

fn sync_prices_command(store: &mut Store) -> anyhow::Result<()> {
    let summary = sync_free_crypto_prices(store)?;
    store.save()?;
    println!(
        "prices updated {}, unsupported crypto assets {}",
        summary.updated, summary.unsupported
    );
    Ok(())
}

fn add_price_command(
    store: &mut Store,
    symbol: &str,
    price: Decimal,
    currency: &str,
) -> anyhow::Result<()> {
    add_manual_price(store, symbol, price, currency)?;
    store.save()?;
    println!("added manual price for {symbol}: {price} {currency}");
    Ok(())
}

fn base_command(store: &mut Store, currency: String) -> anyhow::Result<()> {
    if !store.data.config.base_currencies.contains(&currency) {
        store.data.config.base_currencies.push(currency.clone());
    }
    store.data.config.selected_base_currency = currency.clone();
    store.save()?;
    println!("selected base currency: {currency}");
    Ok(())
}
