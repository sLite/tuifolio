pub mod support;

use rust_decimal::Decimal;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{
    accounting::build_report,
    assets::{AssetInput, save_asset},
    model::{AssetKind, Id, LedgerEffect, TransactionKind},
    price_sync::add_manual_price_for_asset,
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

struct Fixture {
    store: Store,
    portfolio: Id,
    euro: Id,
}

impl Fixture {
    fn new() -> Self {
        let mut store = temp_store();
        let portfolio = create_portfolio(&mut store, "Main");
        let euro = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
        Self {
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

    add_manual_price_for_asset(&mut fixture.store, usd, Decimal::new(9, 1), "EUR").unwrap();
    add_manual_price_for_asset(&mut fixture.store, debt, Decimal::ONE, "EUR").unwrap();
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
    add_manual_price_for_asset(&mut fixture.store, asset, Decimal::ZERO, "EUR").unwrap();
    let report = build_report(&fixture.store.data);
    assert_eq!(report.holdings[0].value, Some(Decimal::ZERO));
    assert!(!report.holdings[0].missing_valuation);
    assert_eq!(report.portfolios[0].unresolved, 0);
    assert_eq!(report.portfolios[0].unresolved_pnl, 0);
    assert_eq!(report.total_unrealized_pnl, Some(Decimal::ZERO));
}
