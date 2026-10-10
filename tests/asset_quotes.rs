pub mod support;

use rust_decimal::Decimal;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{
    accounting::{HoldingRow, build_report},
    assets::{AssetInput, save_asset},
    model::{
        AssetKind, Id, LedgerEffect,
        LedgerEffect::Post,
        LedgerRole, TransactionKind,
        TransactionKind::{AssetDecrease, AssetIncrease},
    },
    price_sync::add_manual_price,
    store::Store,
    transactions::{
        ManualTransactionInput, add_manual_transaction, delete_transaction, update_transaction,
    },
};

struct Fixture {
    store: Store,
    property: Id,
    cash: Id,
    portfolio: Id,
}

impl Fixture {
    fn new() -> Self {
        let mut store = temp_store();
        let property = save_asset(
            &mut store,
            None,
            AssetInput {
                symbol: "HOME".into(),
                name: "Home".into(),
                kind: AssetKind::Property,
                yahoo_symbol: None,
                tradingview_symbol: None,
                valuation_currency: Some("EUR".into()),
            },
        )
        .unwrap();
        let cash = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
        let portfolio = create_portfolio(&mut store, "Main");
        Self {
            store,
            property,
            cash,
            portfolio,
        }
    }

    fn input(
        &self,
        kind: TransactionKind,
        quantity: i64,
        quote: Option<i64>,
        effect: LedgerEffect,
    ) -> ManualTransactionInput {
        ManualTransactionInput {
            portfolio_id: self.portfolio,
            timestamp: chrono::Utc::now(),
            kind,
            base_asset_id: self.property,
            base_amount: Decimal::from(quantity),
            quote_asset_id: quote.map(|_| self.cash),
            quote_amount: quote.map(Decimal::from),
            quote_ledger_effect: effect,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: None,
        }
    }

    fn record(
        &mut self,
        kind: TransactionKind,
        quantity: i64,
        quote: Option<i64>,
        effect: LedgerEffect,
    ) -> Id {
        let input = self.input(kind, quantity, quote, effect);
        add_manual_transaction(&mut self.store, input)
            .unwrap()
            .transaction_id
    }

    fn cash_balance(&self) -> Decimal {
        self.store
            .data
            .ledger_entries
            .iter()
            .filter(|entry| entry.portfolio_id == self.portfolio && entry.asset_id == self.cash)
            .map(|entry| entry.quantity_delta)
            .sum()
    }

    fn acquire(&mut self, quantity: i64, cost: i64) -> Id {
        self.record(
            TransactionKind::AssetIncrease,
            quantity,
            Some(cost),
            LedgerEffect::Ignore,
        )
    }

    fn holding(&self) -> HoldingRow {
        build_report(&self.store.data)
            .holdings
            .into_iter()
            .find(|holding| {
                holding.portfolio_id == self.portfolio && holding.asset_id == self.property
            })
            .unwrap()
    }
}

#[test]
fn property_costs_and_unquoted_valuation_increases_produce_the_expected_pnl() {
    let mut fixture = Fixture::new();
    fixture.acquire(255000, 420000);
    fixture.record(
        TransactionKind::AssetIncrease,
        245000,
        None,
        LedgerEffect::Ignore,
    );
    let holding = fixture.holding();
    assert_eq!(holding.value, Some(Decimal::from(500000)));
    assert_eq!(holding.net_invested, Some(Decimal::from(420000)));
    assert_eq!(holding.unrealized_pnl, Some(Decimal::from(80000)));
    assert_eq!(
        build_report(&fixture.store.data).total_unrealized_pnl,
        Some(Decimal::from(80000))
    );
    assert_eq!(fixture.cash_balance(), Decimal::ZERO);
}

#[test]
fn asset_increases_deduct_cash_only_when_posting_is_selected() {
    for effect in [LedgerEffect::Ignore, LedgerEffect::Post] {
        let mut fixture = Fixture::new();
        fixture.record(TransactionKind::AssetIncrease, 500000, Some(420000), effect);
        let expected = if effect == LedgerEffect::Post {
            -420000
        } else {
            0
        };
        assert_eq!(fixture.cash_balance(), Decimal::from(expected));
        assert_eq!(fixture.holding().net_invested, Some(Decimal::from(420000)));
        assert_eq!(fixture.holding().unrealized_pnl, Some(Decimal::from(80000)));
    }
}

#[test]
fn asset_decreases_apply_proceeds_to_pnl_and_optionally_add_cash() {
    for effect in [LedgerEffect::Ignore, LedgerEffect::Post] {
        let mut fixture = Fixture::new();
        fixture.acquire(500000, 420000);
        fixture.record(TransactionKind::AssetDecrease, 100000, Some(60000), effect);
        let expected = if effect == LedgerEffect::Post {
            60000
        } else {
            0
        };
        assert_eq!(fixture.cash_balance(), Decimal::from(expected));
        assert_eq!(fixture.holding().value, Some(Decimal::from(400000)));
        assert_eq!(fixture.holding().net_invested, Some(Decimal::from(360000)));
        assert_eq!(fixture.holding().unrealized_pnl, Some(Decimal::from(40000)));
    }
}

#[test]
fn unquoted_valuation_adjustments_change_value_without_changing_cost_or_cash() {
    let mut fixture = Fixture::new();
    fixture.acquire(255000, 420000);
    fixture.record(
        TransactionKind::AssetIncrease,
        245000,
        None,
        LedgerEffect::Post,
    );
    fixture.record(
        TransactionKind::AssetDecrease,
        100000,
        None,
        LedgerEffect::Post,
    );
    assert_eq!(fixture.holding().value, Some(Decimal::from(400000)));
    assert_eq!(fixture.holding().net_invested, Some(Decimal::from(420000)));
    assert_eq!(
        fixture.holding().unrealized_pnl,
        Some(Decimal::from(-20000))
    );
    assert_eq!(fixture.cash_balance(), Decimal::ZERO);
}

#[test]
fn changing_the_cash_effect_rebuilds_cash_without_changing_cost_basis() {
    let mut fixture = Fixture::new();
    let id = fixture.acquire(500000, 420000);
    for effect in [LedgerEffect::Post, LedgerEffect::Ignore] {
        let input = fixture.input(TransactionKind::AssetIncrease, 500000, Some(420000), effect);
        update_transaction(&mut fixture.store, id, input).unwrap();
        let expected = if effect == LedgerEffect::Post {
            -420000
        } else {
            0
        };
        assert_eq!(fixture.cash_balance(), Decimal::from(expected));
        assert_eq!(fixture.holding().net_invested, Some(Decimal::from(420000)));
    }
}

#[test]
fn deleting_an_asset_decrease_removes_its_proceeds_and_cash_movement() {
    let mut fixture = Fixture::new();
    fixture.acquire(500000, 420000);
    let id = fixture.record(
        TransactionKind::AssetDecrease,
        100000,
        Some(60000),
        LedgerEffect::Post,
    );
    delete_transaction(&mut fixture.store, id).unwrap();
    assert_eq!(fixture.cash_balance(), Decimal::ZERO);
    assert_eq!(fixture.holding().value, Some(Decimal::from(500000)));
    assert_eq!(fixture.holding().net_invested, Some(Decimal::from(420000)));
    assert_eq!(fixture.holding().unrealized_pnl, Some(Decimal::from(80000)));
}

#[test]
fn asset_quotes_use_historical_exchange_rates_for_cost_basis() {
    let mut fixture = Fixture::new();
    let usd = create_asset(&mut fixture.store, "USD", "US dollar", AssetKind::Fiat);
    add_manual_price(&mut fixture.store, "USD", Decimal::new(9, 1), "EUR").unwrap();
    let mut input = fixture.input(
        TransactionKind::AssetIncrease,
        500000,
        Some(420000),
        LedgerEffect::Ignore,
    );
    input.quote_asset_id = Some(usd);
    add_manual_transaction(&mut fixture.store, input).unwrap();
    add_manual_price(&mut fixture.store, "USD", Decimal::new(8, 1), "EUR").unwrap();
    assert_eq!(fixture.holding().net_invested, Some(Decimal::from(378000)));
    assert_eq!(
        fixture.holding().unrealized_pnl,
        Some(Decimal::from(122000))
    );
}

#[test]
fn quoted_movements_are_scoped_to_the_holding_portfolio_and_asset() {
    let mut fixture = Fixture::new();
    fixture.acquire(500000, 420000);
    let other_portfolio = create_portfolio(&mut fixture.store, "Other");
    let other_asset = create_asset(
        &mut fixture.store,
        "OTHER",
        "Other asset",
        AssetKind::Custom,
    );
    let mut elsewhere = fixture.input(
        TransactionKind::AssetIncrease,
        100000,
        Some(200000),
        LedgerEffect::Post,
    );
    elsewhere.portfolio_id = other_portfolio;
    add_manual_transaction(&mut fixture.store, elsewhere).unwrap();
    let mut different_asset = fixture.input(
        TransactionKind::AssetIncrease,
        100000,
        Some(300000),
        LedgerEffect::Ignore,
    );
    different_asset.base_asset_id = other_asset;
    add_manual_transaction(&mut fixture.store, different_asset).unwrap();
    assert_eq!(fixture.holding().net_invested, Some(Decimal::from(420000)));
    assert_eq!(fixture.cash_balance(), Decimal::ZERO);
}

#[test]
fn quoted_asset_movements_work_for_market_priced_assets() {
    let mut fixture = Fixture::new();
    let stock = create_asset(&mut fixture.store, "AAPL", "Apple", AssetKind::Stock);
    add_manual_price(&mut fixture.store, "AAPL", Decimal::from(100), "EUR").unwrap();
    let mut acquisition = fixture.input(AssetIncrease, 10, Some(800), Post);
    acquisition.base_asset_id = stock;
    add_manual_transaction(&mut fixture.store, acquisition).unwrap();
    let mut disposal = fixture.input(AssetDecrease, 4, Some(500), Post);
    disposal.base_asset_id = stock;
    add_manual_transaction(&mut fixture.store, disposal).unwrap();
    let report = build_report(&fixture.store.data);
    let holding = report
        .holdings
        .iter()
        .find(|holding| holding.asset_id == stock)
        .unwrap();
    assert_eq!(holding.value, Some(Decimal::from(600)));
    assert_eq!(holding.net_invested, Some(Decimal::from(300)));
    assert_eq!(holding.unrealized_pnl, Some(Decimal::from(300)));
    assert_eq!(fixture.cash_balance(), Decimal::from(-300));
}

#[test]
fn reopening_rebuilds_cash_for_previously_saved_asset_quotes() {
    let mut fixture = Fixture::new();
    fixture.record(AssetIncrease, 500000, Some(420000), Post);
    fixture.record(AssetDecrease, 100000, Some(60000), Post);
    fixture
        .store
        .data
        .ledger_entries
        .retain(|entry| !matches!(entry.role, LedgerRole::Quote));
    let transactions = serde_json::to_value(&fixture.store.data.transactions).unwrap();
    let path = fixture.store.path().clone();
    let cash = fixture.cash;
    fixture.store.save().unwrap();
    drop(fixture);
    let reopened = Store::open(Some(path)).unwrap();
    let cash_balance: Decimal = reopened
        .data
        .ledger_entries
        .iter()
        .filter(|entry| entry.asset_id == cash)
        .map(|entry| entry.quantity_delta)
        .sum();
    assert_eq!(cash_balance, Decimal::from(-360000));
    assert_eq!(reopened.data.ledger_entries.len(), 4);
    assert_eq!(
        serde_json::to_value(&reopened.data.transactions).unwrap(),
        transactions
    );
}
