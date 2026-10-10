pub mod support;

use rust_decimal::Decimal;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{
    accounting::{HoldingRow, build_report},
    assets::{AssetInput, save_asset},
    model::{AssetKind, Id, LedgerEffect, LedgerRole, StockSplit, TransactionKind},
    price_sync::add_manual_price,
    store::Store,
    transactions::{
        ManualTransactionInput, add_manual_transaction, delete_transaction, update_transaction,
    },
};

struct Fixture {
    store: Store,
    asset: Id,
    cash: Id,
    portfolio: Id,
}

impl Fixture {
    fn new() -> Self {
        let mut store = temp_store();
        let asset = save_asset(
            &mut store,
            None,
            AssetInput {
                symbol: "HOME".into(),
                name: "Gifted home".into(),
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
            asset,
            cash,
            portfolio,
        }
    }

    fn input(&self) -> ManualTransactionInput {
        ManualTransactionInput {
            portfolio_id: self.portfolio,
            timestamp: chrono::Utc::now(),
            kind: TransactionKind::Gift,
            base_asset_id: self.asset,
            base_amount: Decimal::from(350000),
            quote_asset_id: None,
            quote_amount: None,
            quote_ledger_effect: LedgerEffect::Post,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: None,
        }
    }

    fn record(&mut self) -> Id {
        let input = self.input();
        add_manual_transaction(&mut self.store, input)
            .unwrap()
            .transaction_id
    }

    fn holding(&self) -> HoldingRow {
        build_report(&self.store.data)
            .holdings
            .into_iter()
            .find(|holding| {
                holding.asset_id == self.asset && holding.portfolio_id == self.portfolio
            })
            .unwrap()
    }
}

#[test]
fn gifts_record_incoming_quantity_and_implicit_zero_cost_without_a_quote_currency() {
    let mut fixture = Fixture::new();
    fixture.record();
    let transaction = &fixture.store.data.transactions[0];
    assert_eq!(transaction.kind, TransactionKind::Gift);
    assert_eq!(transaction.quote_asset_id, None);
    assert_eq!(transaction.quote_amount, None);
    assert_eq!(transaction.quote_ledger_effect, LedgerEffect::Ignore);
    assert_eq!(fixture.store.data.ledger_entries.len(), 1);
    assert_eq!(fixture.holding().quantity, Decimal::from(350000));
    assert_eq!(fixture.holding().net_invested, Some(Decimal::ZERO));
    assert_eq!(
        fixture.holding().unrealized_pnl,
        Some(Decimal::from(350000))
    );
    assert!(fixture.store.data.prices.is_empty());
}

#[test]
fn gifts_do_not_erase_costs_from_paid_acquisitions_of_the_same_asset() {
    let mut fixture = Fixture::new();
    let mut paid = fixture.input();
    paid.kind = TransactionKind::Buy;
    paid.base_amount = Decimal::from(50000);
    paid.quote_asset_id = Some(fixture.cash);
    paid.quote_amount = Some(Decimal::from(40000));
    paid.quote_ledger_effect = LedgerEffect::Ignore;
    add_manual_transaction(&mut fixture.store, paid).unwrap();
    fixture.record();
    assert_eq!(fixture.holding().value, Some(Decimal::from(400000)));
    assert_eq!(fixture.holding().net_invested, Some(Decimal::from(40000)));
    assert_eq!(
        fixture.holding().unrealized_pnl,
        Some(Decimal::from(360000))
    );
}

#[test]
fn gifts_establish_zero_cost_only_for_their_own_portfolio_and_asset() {
    let mut fixture = Fixture::new();
    fixture.record();
    let other = create_portfolio(&mut fixture.store, "Other");
    let other_asset = create_asset(&mut fixture.store, "OTHER", "Other", AssetKind::Custom);
    for (portfolio, asset) in [(other, fixture.asset), (fixture.portfolio, other_asset)] {
        let mut deposit = fixture.input();
        deposit.kind = TransactionKind::Deposit;
        deposit.portfolio_id = portfolio;
        deposit.base_asset_id = asset;
        add_manual_transaction(&mut fixture.store, deposit).unwrap();
    }
    let report = build_report(&fixture.store.data);
    for holding in report.holdings.iter().filter(|holding| {
        holding.portfolio_id != fixture.portfolio || holding.asset_id != fixture.asset
    }) {
        assert_eq!(holding.net_invested, None);
    }
    assert_eq!(fixture.holding().net_invested, Some(Decimal::ZERO));
    assert_eq!(report.total_unrealized_pnl, Some(Decimal::from(350000)));
}

#[test]
fn gift_fees_post_separately_from_the_implicit_zero_quote() {
    let mut fixture = Fixture::new();
    let mut input = fixture.input();
    input.fee_asset_id = Some(fixture.cash);
    input.fee_amount = Some(Decimal::from(2));
    add_manual_transaction(&mut fixture.store, input).unwrap();
    assert_eq!(fixture.store.data.ledger_entries.len(), 2);
    let fee = &fixture.store.data.ledger_entries[1];
    assert!(matches!(fee.role, LedgerRole::Fee));
    assert_eq!(fee.asset_id, fixture.cash);
    assert_eq!(fee.quantity_delta, Decimal::from(-2));
    assert_eq!(fixture.holding().net_invested, Some(Decimal::ZERO));
    assert_eq!(
        fixture.holding().unrealized_pnl,
        Some(Decimal::from(350000))
    );
}

#[test]
fn changing_a_paid_acquisition_to_gift_removes_its_quote_and_preserves_identity() {
    let mut fixture = Fixture::new();
    let mut paid = fixture.input();
    paid.kind = TransactionKind::Buy;
    paid.quote_asset_id = Some(fixture.cash);
    paid.quote_amount = Some(Decimal::from(100000));
    let id = add_manual_transaction(&mut fixture.store, paid)
        .unwrap()
        .transaction_id;
    let previous = fixture.store.data.transactions[0].clone();
    assert_eq!(fixture.store.data.ledger_entries.len(), 2);
    let gift = fixture.input();
    update_transaction(&mut fixture.store, id, gift).unwrap();
    assert_eq!(fixture.store.data.transactions[0].id, previous.id);
    assert_eq!(
        fixture.store.data.transactions[0].source_row_hash,
        previous.source_row_hash
    );
    assert_eq!(fixture.store.data.ledger_entries.len(), 1);
    assert_eq!(fixture.holding().net_invested, Some(Decimal::ZERO));
}

#[test]
fn changing_a_gift_to_an_unquoted_movement_restores_unknown_cost_basis() {
    let mut fixture = Fixture::new();
    let id = fixture.record();
    let mut input = fixture.input();
    input.kind = TransactionKind::AssetIncrease;
    update_transaction(&mut fixture.store, id, input).unwrap();
    assert_eq!(fixture.holding().net_invested, None);
    assert_eq!(fixture.holding().unrealized_pnl, None);
}

#[test]
fn gifts_survive_save_and_reopen_with_zero_cost_basis_and_no_cash_entry() {
    let mut fixture = Fixture::new();
    fixture.record();
    fixture.store.save().unwrap();
    let path = fixture.store.path().clone();
    drop(fixture);
    let reopened = Store::open(Some(path)).unwrap();
    assert_eq!(reopened.data.transactions[0].kind, TransactionKind::Gift);
    assert_eq!(reopened.data.ledger_entries.len(), 1);
    assert_eq!(
        build_report(&reopened.data).holdings[0].net_invested,
        Some(Decimal::ZERO)
    );
}

#[test]
fn gifted_stock_quantities_follow_split_adjustments_and_market_valuation() {
    let mut fixture = Fixture::new();
    let stock = create_asset(&mut fixture.store, "ABC", "Stock", AssetKind::Stock);
    fixture.store.data.config.stock_splits.push(StockSplit {
        asset_id: stock,
        effective_date: "2024-01-01".into(),
        numerator: Decimal::from(2),
        denominator: Decimal::ONE,
    });
    let mut input = fixture.input();
    input.base_asset_id = stock;
    input.base_amount = Decimal::from(10);
    input.timestamp = chrono::DateTime::parse_from_rfc3339("2023-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    add_manual_transaction(&mut fixture.store, input).unwrap();
    add_manual_price(&mut fixture.store, "ABC", Decimal::from(100), "EUR").unwrap();
    let report = build_report(&fixture.store.data);
    let holding = &report.holdings[0];
    assert_eq!(holding.quantity, Decimal::from(20));
    assert_eq!(holding.net_invested, Some(Decimal::ZERO));
    assert_eq!(holding.unrealized_pnl, Some(Decimal::from(2000)));
}

#[test]
fn gifts_reject_manually_supplied_quotes_without_mutating_data() {
    let mut fixture = Fixture::new();
    let before = serde_json::to_value(&fixture.store.data).unwrap();
    for (asset, amount) in [
        (Some(fixture.cash), None),
        (None, Some(Decimal::ZERO)),
        (Some(fixture.cash), Some(Decimal::from(100))),
    ] {
        let mut input = fixture.input();
        input.quote_asset_id = asset;
        input.quote_amount = amount;
        assert!(add_manual_transaction(&mut fixture.store, input).is_err());
        assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), before);
    }
}

#[test]
fn gifts_require_a_positive_quantity() {
    let mut fixture = Fixture::new();
    for quantity in [Decimal::ZERO, Decimal::from(-1)] {
        let mut input = fixture.input();
        input.base_amount = quantity;
        assert!(add_manual_transaction(&mut fixture.store, input).is_err());
        assert!(fixture.store.data.transactions.is_empty());
    }
}

#[test]
fn deleting_a_gift_removes_its_asset_and_fee_movements() {
    let mut fixture = Fixture::new();
    let mut input = fixture.input();
    input.fee_asset_id = Some(fixture.cash);
    input.fee_amount = Some(Decimal::from(2));
    let id = add_manual_transaction(&mut fixture.store, input)
        .unwrap()
        .transaction_id;
    delete_transaction(&mut fixture.store, id).unwrap();
    assert!(fixture.store.data.transactions.is_empty());
    assert!(fixture.store.data.ledger_entries.is_empty());
    assert_eq!(
        build_report(&fixture.store.data).total_unrealized_pnl,
        Some(Decimal::ZERO)
    );
}
