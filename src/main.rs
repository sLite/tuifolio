use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rust_decimal::Decimal;

use tuifolio::accounting::build_report;
use tuifolio::formatting::money;
use tuifolio::price_sync::{add_manual_price_at, sync_prices};
use tuifolio::store::Store;
use tuifolio::web;

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
    Web {
        #[arg(long, default_value_t = 3000)]
        port: u16,
        #[arg(long, default_value_t = web::DEFAULT_PRICE_REFRESH_SECONDS, help = "Seconds between automatic price refreshes; 0 disables them")]
        price_refresh_seconds: u64,
    },
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
        #[arg(
            long,
            help = "Historical observation time in RFC 3339 format; defaults to now"
        )]
        at: Option<chrono::DateTime<chrono::Utc>>,
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
    let store = Store::open(cli.store)?;
    let command = cli.command.unwrap_or(Command::Web {
        port: 3000,
        price_refresh_seconds: web::DEFAULT_PRICE_REFRESH_SECONDS,
    });
    execute_command(store, command)
}

fn execute_command(mut store: Store, command: Command) -> anyhow::Result<()> {
    match command {
        Command::Web {
            port,
            price_refresh_seconds,
        } => {
            return web::run(
                store,
                web::WebOptions {
                    port,
                    price_refresh_interval: std::time::Duration::from_secs(price_refresh_seconds),
                },
            );
        }
        Command::Summary => summary_command(&store),
        Command::Holding { symbol } => holding_command(&store, &symbol),
        Command::MissingPrices => missing_prices_command(&store),
        Command::SyncPrices => sync_prices_command(&mut store)?,
        Command::AddPrice {
            symbol,
            price,
            currency,
            at,
        } => add_price_command(&mut store, &symbol, price, &currency, at)?,
        Command::Base { currency } => base_command(&mut store, currency)?,
    }
    Ok(())
}

fn summary_command(store: &Store) {
    let report = build_report(&store.data);
    println!("Base: {}", report.base_currency);
    let missing = report
        .holdings
        .iter()
        .filter(|holding| holding.missing_valuation)
        .count();
    let partial = if missing > 0 { " (partial)" } else { "" };
    println!("Assets{partial}: {}", money(report.total_assets));
    println!("Liabilities{partial}: {}", money(report.total_liabilities));
    println!("Net worth{partial}: {}", money(report.net_value));
    println!(
        "Unrealized PnL: {}{}",
        report
            .total_unrealized_pnl
            .map(money)
            .unwrap_or_else(|| "Unavailable".into()),
        if report.unresolved_pnl > 0 && report.total_unrealized_pnl.is_some() {
            " (partial)"
        } else {
            ""
        }
    );
    if missing > 0 {
        println!(
            "Warning: {missing} holdings have no valuation. Partial totals exclude these holdings, including unvalued cash and liabilities."
        );
    }
    if report.unresolved_pnl > 0 {
        println!(
            "Warning: {} investment holdings have unavailable PnL because valuation or cost basis is missing.",
            report.unresolved_pnl
        );
    }
    println!(
        "Realized PnL: {}{}",
        report
            .total_realized_pnl
            .map(money)
            .unwrap_or_else(|| "Unavailable".into()),
        if report.unresolved_realized_pnl > 0 && report.total_realized_pnl.is_some() {
            " (partial)"
        } else {
            ""
        }
    );
    if report.unresolved_realized_pnl > 0 {
        println!(
            "Warning: {} investment positions have unavailable realized PnL.",
            report.unresolved_realized_pnl
        );
    }
    for row in &report.realized_returns {
        println!(
            "Realized {} {}: {}",
            row.portfolio,
            row.symbol,
            row.pnl.map(money).unwrap_or_else(|| "Unavailable".into())
        );
    }
    println!(
        "Staking income: {}{}",
        report
            .total_staking_income
            .map(money)
            .unwrap_or_else(|| "Unavailable".into()),
        if report.unresolved_staking_income > 0 && report.total_staking_income.is_some() {
            " (partial)"
        } else {
            ""
        }
    );
    if report.unresolved_staking_income > 0 {
        println!(
            "Warning: {} staking positions lack historical prices or conversions.",
            report.unresolved_staking_income
        );
    }
    for row in &report.staking_returns {
        println!(
            "Staking {} {}: {}",
            row.portfolio,
            row.symbol,
            row.income
                .map(money)
                .unwrap_or_else(|| "Unavailable".into())
        );
    }
    println!("Holdings: {}", report.holdings.len());
    println!("Negative balances: {}", report.negative_balances.len());
    for portfolio in report.portfolios {
        println!(
            "{}{}: assets={} liabilities={} net={} missing={} pnl_missing={}",
            portfolio.name,
            if portfolio.unresolved > 0 {
                " (partial)"
            } else {
                ""
            },
            money(portfolio.assets),
            money(portfolio.liabilities),
            money(portfolio.net_value),
            portfolio.unresolved,
            portfolio.unresolved_pnl
        );
    }
}

fn missing_prices_command(store: &Store) {
    let report = build_report(&store.data);
    for holding in report
        .holdings
        .iter()
        .filter(|holding| holding.missing_valuation)
    {
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
            "{} {} quantity={} value={} {} remaining_basis={} unrealized_pnl={} realized_pnl={}",
            holding.portfolio,
            holding.symbol,
            holding.quantity,
            holding.value.map(money).unwrap_or_else(|| "n/a".into()),
            report.base_currency,
            holding
                .remaining_cost_basis
                .map(money)
                .unwrap_or_else(|| "Unavailable".into()),
            holding
                .unrealized_pnl
                .map(money)
                .unwrap_or_else(|| "Unavailable".into()),
            holding
                .realized_pnl
                .map(money)
                .unwrap_or_else(|| "Unavailable".into())
        );
    }
}

fn sync_prices_command(store: &mut Store) -> anyhow::Result<()> {
    let summary = sync_prices(store)?;
    store.save()?;
    println!(
        "prices updated {}, unsupported assets {}",
        summary.updated, summary.unsupported
    );
    Ok(())
}

fn add_price_command(
    store: &mut Store,
    symbol: &str,
    price: Decimal,
    currency: &str,
    at: Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<()> {
    add_manual_price_at(
        store,
        symbol,
        price,
        currency,
        at.unwrap_or_else(chrono::Utc::now),
    )?;
    store.save()?;
    println!("added manual price for {symbol}: {price} {currency}");
    Ok(())
}

fn base_command(store: &mut Store, currency: String) -> anyhow::Result<()> {
    let currency = tuifolio::currencies::normalize_reporting_currency(&store.data, &currency)?;
    if !store.data.config.base_currencies.contains(&currency) {
        store.data.config.base_currencies.push(currency.clone());
    }
    store.data.config.selected_base_currency = currency.clone();
    store.save()?;
    println!("selected base currency: {currency}");
    Ok(())
}
