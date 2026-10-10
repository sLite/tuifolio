pub mod support;

use std::{path::PathBuf, process::Command};

use rust_decimal::Decimal;
use support::{create_asset, create_portfolio};
use tuifolio::{
    accounting::build_report,
    assets::{AssetInput, save_asset},
    model::{AssetKind, Id, LedgerEffect, Price, TransactionKind},
    price_sync::add_manual_price,
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

struct Fixture {
    directory: tempfile::TempDir,
    path: PathBuf,
    store: Store,
    portfolio: Id,
    euro: Id,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.json");
        let mut store = Store::open(Some(path.clone())).unwrap();
        let portfolio = create_portfolio(&mut store, "Main");
        let euro = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
        Self {
            directory,
            path,
            store,
            portfolio,
            euro,
        }
    }

    fn asset(&mut self, symbol: &str, kind: AssetKind, currency: Option<&str>) -> Id {
        save_asset(
            &mut self.store,
            None,
            AssetInput {
                symbol: symbol.into(),
                name: symbol.into(),
                kind,
                yahoo_symbol: None,
                tradingview_symbol: None,
                valuation_currency: currency.map(str::to_string),
            },
        )
        .unwrap()
    }

    fn post(&mut self, asset: Id, kind: TransactionKind, quantity: i64, cost: Option<i64>) {
        add_manual_transaction(
            &mut self.store,
            ManualTransactionInput {
                portfolio_id: self.portfolio,
                timestamp: chrono::Utc::now(),
                kind,
                base_asset_id: asset,
                base_amount: Decimal::from(quantity),
                quote_asset_id: cost.map(|_| self.euro),
                quote_amount: cost.map(Decimal::from),
                quote_ledger_effect: LedgerEffect::Ignore,
                fee_asset_id: None,
                fee_amount: None,
                exchange: None,
                broker: None,
                notes: None,
            },
        )
        .unwrap();
    }

    fn cli(self, commands: &[&str]) -> Vec<String> {
        self.store.save().unwrap();
        drop(self.store);
        let original = std::fs::read(&self.path).unwrap();
        let output = commands
            .iter()
            .map(|command| {
                let output = Command::new(env!("CARGO_BIN_EXE_tuifolio"))
                    .args(["--store", self.path.to_str().unwrap(), command])
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(std::fs::read(&self.path).unwrap(), original);
                String::from_utf8(output.stdout).unwrap()
            })
            .collect();
        drop(self.directory);
        output
    }
}

#[test]
fn unvalued_cash_and_both_market_and_intrinsic_debt_count_until_rates_are_supplied() {
    let mut fixture = Fixture::new();
    let usd = fixture.asset("USD", AssetKind::Fiat, None);
    let debt = fixture.asset("DEBT", AssetKind::Liability, None);
    let foreign_debt = fixture.asset("FOREIGNDEBT", AssetKind::Liability, Some("USD"));
    let settled = fixture.asset("SETTLED", AssetKind::Liability, None);
    fixture.post(fixture.euro, TransactionKind::Deposit, 1000, None);
    fixture.post(usd, TransactionKind::Deposit, 100, None);
    fixture.post(debt, TransactionKind::LiabilityIncrease, 100, None);
    fixture.post(foreign_debt, TransactionKind::LiabilityIncrease, 2000, None);
    fixture.post(settled, TransactionKind::LiabilityIncrease, 5, None);
    fixture.post(settled, TransactionKind::LiabilityDecrease, 5, None);
    let original = serde_json::to_value(&fixture.store.data).unwrap();
    let report = build_report(&fixture.store.data);
    assert_eq!(report.holdings.len(), 4);
    assert_eq!(report.portfolios[0].unresolved, 3);
    assert_eq!(report.total_assets, Decimal::from(1000));
    assert_eq!(report.total_liabilities, Decimal::ZERO);
    assert_eq!(report.net_value, Decimal::from(1000));
    assert_eq!(
        report.unresolved_pnl, 0,
        "cash and debt do not require acquisition basis"
    );
    for holding in &report.holdings {
        assert_eq!(holding.missing_valuation, holding.value.is_none());
    }
    assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), original);

    add_manual_price(&mut fixture.store, "USD", Decimal::new(9, 1), "EUR").unwrap();
    add_manual_price(&mut fixture.store, "DEBT", Decimal::ONE, "EUR").unwrap();
    let report = build_report(&fixture.store.data);
    assert_eq!(report.portfolios[0].unresolved, 0);
    assert!(
        report
            .holdings
            .iter()
            .all(|holding| !holding.missing_valuation)
    );
    assert_eq!(report.total_assets, Decimal::from(1090));
    assert_eq!(report.total_liabilities, Decimal::from(1900));
    assert_eq!(report.net_value, Decimal::from(-810));
}

#[test]
fn empty_or_cash_only_portfolios_cannot_turn_unknown_investment_pnl_into_zero() {
    let mut fixture = Fixture::new();
    let unknown = fixture.asset("UNKNOWN", AssetKind::Custom, Some("EUR"));
    fixture.post(unknown, TransactionKind::AssetIncrease, 100, None);
    create_portfolio(&mut fixture.store, "Empty");
    let cash_only = create_portfolio(&mut fixture.store, "Cash only");
    let investment = fixture.portfolio;
    fixture.portfolio = cash_only;
    fixture.post(fixture.euro, TransactionKind::Deposit, 200, None);
    fixture.portfolio = investment;
    let report = build_report(&fixture.store.data);
    assert_eq!(report.total_unrealized_pnl, None);
    assert_eq!(report.unresolved_pnl, 1);
    assert_eq!(report.portfolios[0].unrealized_pnl, None);
    assert_eq!(report.portfolios[0].unresolved, 0);
    assert_eq!(report.portfolios[0].unresolved_pnl, 1);
    assert!(
        report.portfolios[1..]
            .iter()
            .all(|portfolio| portfolio.unrealized_pnl == Some(Decimal::ZERO))
    );

    let known = fixture.asset("KNOWN", AssetKind::Custom, Some("EUR"));
    fixture.post(known, TransactionKind::AssetIncrease, 100, Some(100));
    let report = build_report(&fixture.store.data);
    assert_eq!(report.total_unrealized_pnl, Some(Decimal::ZERO));
    assert_eq!(
        report.unresolved_pnl, 1,
        "known zero is still only a partial PnL"
    );
}

#[test]
fn a_recorded_zero_market_value_is_available_and_has_known_zero_pnl() {
    let mut fixture = Fixture::new();
    let asset = fixture.asset("WORTHLESS", AssetKind::Stock, None);
    fixture.post(asset, TransactionKind::Gift, 10, None);
    // Stored zero observations can already exist; manual zero-price support is finding 16.
    fixture.store.data.prices.push(Price {
        asset_id: asset,
        timestamp: chrono::Utc::now(),
        price: Decimal::ZERO,
        currency: "EUR".into(),
        source: "fixture".into(),
    });
    let report = build_report(&fixture.store.data);
    assert_eq!(report.holdings[0].value, Some(Decimal::ZERO));
    assert!(!report.holdings[0].missing_valuation);
    assert_eq!(report.portfolios[0].unresolved, 0);
    assert_eq!(report.portfolios[0].unresolved_pnl, 0);
    assert_eq!(report.total_unrealized_pnl, Some(Decimal::ZERO));
}

#[test]
fn cli_summary_and_missing_prices_include_unvalued_cash_and_liabilities_without_saving() {
    let mut fixture = Fixture::new();
    let usd = fixture.asset("USD", AssetKind::Fiat, None);
    let debt = fixture.asset("DEBT", AssetKind::Liability, None);
    let unknown = fixture.asset("UNKNOWN", AssetKind::Custom, Some("EUR"));
    fixture.post(fixture.euro, TransactionKind::Deposit, 1000, None);
    fixture.post(usd, TransactionKind::Deposit, 100, None);
    fixture.post(debt, TransactionKind::LiabilityIncrease, 20, None);
    fixture.post(unknown, TransactionKind::AssetIncrease, 100, None);
    let output = fixture.cli(&["summary", "missing-prices"]);
    assert!(output[0].contains("Assets (partial): 1100.00"));
    assert!(output[0].contains("Liabilities (partial): 0.00"));
    assert!(output[0].contains("Net worth (partial): 1100.00"));
    assert!(output[0].contains("Unrealized PnL: Unavailable"));
    assert!(output[0].contains("2 holdings have no valuation"));
    assert!(output[0].contains("missing=2 pnl_missing=1"));
    assert!(output[1].contains("Main USD (Fiat)"));
    assert!(output[1].contains("Main DEBT (Liability)"));
    assert!(!output[1].contains("EUR"));
    assert!(!output[1].contains("UNKNOWN"));
}

#[test]
fn cli_distinguishes_unknown_partial_and_complete_zero_pnl() {
    for (known, unknown, expected) in [
        (false, true, "Unrealized PnL: Unavailable"),
        (true, true, "Unrealized PnL: 0.00 (partial)"),
        (true, false, "Unrealized PnL: 0.00\n"),
    ] {
        let mut fixture = Fixture::new();
        if known {
            let asset = fixture.asset("KNOWN", AssetKind::Custom, Some("EUR"));
            fixture.post(asset, TransactionKind::AssetIncrease, 100, Some(100));
        }
        if unknown {
            let asset = fixture.asset("UNKNOWN", AssetKind::Custom, Some("EUR"));
            fixture.post(asset, TransactionKind::AssetIncrease, 100, None);
        }
        let output = fixture.cli(&["summary"]);
        assert!(output[0].contains(expected), "{}", output[0]);
        assert_eq!(
            output[0].contains("investment holdings have unavailable PnL"),
            unknown
        );
        assert!(!output[0].contains("Net worth (partial)"));
    }
}
