use rust_decimal::Decimal;
use tuifolio::{
    ledger::rebuild_ledger,
    model::{AssetKind, Id, LedgerEffect, TransactionKind},
    store::Store,
    transactions::{
        ManualTransactionInput, add_manual_transaction, delete_transaction, update_transaction,
    },
};

struct Fixture {
    _directory: tempfile::TempDir,
    store: Store,
    id: Id,
    other_asset: Id,
    other_portfolio: Id,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(Some(directory.path().join("store.json"))).unwrap();
        let base = support::create_asset(&mut store, "BTC", "Bitcoin", AssetKind::Crypto);
        let quote = support::create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
        let fee = support::create_asset(&mut store, "USD", "US dollar", AssetKind::Fiat);
        let other_asset = support::create_asset(&mut store, "ETH", "Ethereum", AssetKind::Crypto);
        let portfolio = support::create_portfolio(&mut store, "Main");
        let other_portfolio = support::create_portfolio(&mut store, "Other");
        let input = initial_input(portfolio, base, quote, fee);
        let id = add_manual_transaction(&mut store, input)
            .unwrap()
            .transaction_id;
        Self {
            _directory: directory,
            store,
            id,
            other_asset,
            other_portfolio,
        }
    }

    fn input(&self) -> ManualTransactionInput {
        ManualTransactionInput::from(
            self.store
                .data
                .transactions
                .iter()
                .find(|transaction| transaction.id == self.id)
                .unwrap(),
        )
    }
}

fn initial_input(
    portfolio_id: Id,
    base_asset_id: Id,
    quote: Id,
    fee: Id,
) -> ManualTransactionInput {
    ManualTransactionInput {
        portfolio_id,
        timestamp: chrono::Utc::now(),
        kind: TransactionKind::Buy,
        base_asset_id,
        base_amount: Decimal::ONE,
        quote_asset_id: Some(quote),
        quote_amount: Some(Decimal::from(100)),
        quote_ledger_effect: LedgerEffect::Post,
        fee_asset_id: Some(fee),
        fee_amount: Some(Decimal::from(2)),
        exchange: None,
        broker: None,
        notes: None,
    }
}

fn balance(store: &Store, portfolio: Id, asset: Id) -> Decimal {
    store
        .data
        .ledger_entries
        .iter()
        .filter(|entry| entry.portfolio_id == portfolio && entry.asset_id == asset)
        .map(|entry| entry.quantity_delta)
        .sum()
}

#[test]
fn updating_replaces_ledger_effects_and_keeps_the_transaction_identity_and_origin() {
    let mut fixture = Fixture::new();
    let previous = fixture.store.data.transactions[0].clone();
    let next_id = fixture.store.data.next_id;
    let mut input = fixture.input();
    input.portfolio_id = fixture.other_portfolio;
    input.base_asset_id = fixture.other_asset;
    input.kind = TransactionKind::Sell;
    input.base_amount = Decimal::from(2);
    input.quote_amount = Some(Decimal::from(250));
    input.fee_amount = Some(Decimal::from(3));
    input.exchange = Some("Exchange".into());
    input.broker = Some("Broker".into());
    input.notes = Some("Updated".into());
    update_transaction(&mut fixture.store, fixture.id, input).unwrap();
    assert_eq!(fixture.store.data.transactions.len(), 1);
    let updated = &fixture.store.data.transactions[0];
    assert_eq!(
        (updated.id, &updated.source, &updated.source_row_hash),
        (previous.id, &previous.source, &previous.source_row_hash)
    );
    assert_eq!(fixture.store.data.next_id, next_id);
    assert_moved_balances(&fixture, &previous);
}

fn assert_moved_balances(fixture: &Fixture, previous: &tuifolio::model::Transaction) {
    assert_eq!(
        balance(
            &fixture.store,
            previous.portfolio_id,
            previous.base_asset_id
        ),
        Decimal::ZERO
    );
    assert_eq!(
        balance(&fixture.store, fixture.other_portfolio, fixture.other_asset),
        Decimal::from(-2)
    );
    assert_eq!(
        balance(
            &fixture.store,
            fixture.other_portfolio,
            previous.quote_asset_id.unwrap()
        ),
        Decimal::from(250)
    );
    assert_eq!(
        balance(
            &fixture.store,
            fixture.other_portfolio,
            previous.fee_asset_id.unwrap()
        ),
        Decimal::from(-3)
    );
}

#[test]
fn deleting_removes_all_sides_without_deleting_assets_or_portfolios() {
    let mut fixture = Fixture::new();
    let assets = serde_json::to_value(&fixture.store.data.assets).unwrap();
    let portfolios = serde_json::to_value(&fixture.store.data.portfolios).unwrap();
    let next_id = fixture.store.data.next_id;
    delete_transaction(&mut fixture.store, fixture.id).unwrap();
    assert!(fixture.store.data.transactions.is_empty());
    assert!(fixture.store.data.ledger_entries.is_empty());
    assert_eq!(
        serde_json::to_value(&fixture.store.data.assets).unwrap(),
        assets
    );
    assert_eq!(
        serde_json::to_value(&fixture.store.data.portfolios).unwrap(),
        portfolios
    );
    assert_eq!(fixture.store.data.next_id, next_id);
}

#[test]
fn invalid_or_missing_updates_and_deletions_are_atomic() {
    let mut fixture = Fixture::new();
    let before = serde_json::to_value(&fixture.store.data).unwrap();
    let mut input = fixture.input();
    input.base_amount = Decimal::from(-1);
    assert!(update_transaction(&mut fixture.store, fixture.id, input).is_err());
    let input = fixture.input();
    assert!(update_transaction(&mut fixture.store, 999, input).is_err());
    assert!(delete_transaction(&mut fixture.store, 999).is_err());
    assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), before);
}

#[test]
fn ledger_rebuild_failure_rolls_back_updates_and_deletions() {
    let mut fixture = Fixture::new();
    let mut broken = fixture.store.data.transactions[0].clone();
    broken.id = 999;
    broken.base_asset_id = 999;
    fixture.store.data.transactions.push(broken);
    let before = serde_json::to_value(&fixture.store.data).unwrap();
    let mut input = fixture.input();
    input.base_amount = Decimal::from(2);
    assert!(update_transaction(&mut fixture.store, fixture.id, input).is_err());
    assert!(delete_transaction(&mut fixture.store, fixture.id).is_err());
    assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), before);
    delete_transaction(&mut fixture.store, 999).unwrap();
    assert_eq!(fixture.store.data.transactions.len(), 1);
}

#[test]
fn cost_basis_only_preserves_asset_and_fee_movements_until_cash_effect_changes() {
    let mut fixture = Fixture::new();
    fixture.store.data.transactions[0].quote_ledger_effect = LedgerEffect::Ignore;
    rebuild_ledger(&mut fixture.store.data).unwrap();
    let mut input = fixture.input();
    input.notes = Some("Changed notes".into());
    update_transaction(&mut fixture.store, fixture.id, input).unwrap();
    let transaction = &fixture.store.data.transactions[0];
    assert_eq!(fixture.store.data.ledger_entries.len(), 2);
    for (asset, amount) in [
        (transaction.base_asset_id, 1),
        (transaction.quote_asset_id.unwrap(), 0),
        (transaction.fee_asset_id.unwrap(), -2),
    ] {
        assert_eq!(
            balance(&fixture.store, transaction.portfolio_id, asset),
            Decimal::from(amount)
        );
    }
    let mut input = fixture.input();
    input.quote_ledger_effect = LedgerEffect::Post;
    update_transaction(&mut fixture.store, fixture.id, input).unwrap();
    assert_eq!(fixture.store.data.ledger_entries.len(), 3);
}

#[derive(Clone, Copy, Debug)]
enum InvalidAmount {
    ZeroBase,
    NegativeBase,
    MissingQuoteAsset,
    MissingQuoteAmount,
    NegativeQuote,
    ZeroFee,
    NegativeFee,
    MissingFeeAsset,
    MissingFeeAmount,
}

fn invalidate(transaction: &mut tuifolio::model::Transaction, case: InvalidAmount) {
    match case {
        InvalidAmount::ZeroBase => transaction.base_amount = Decimal::ZERO,
        InvalidAmount::NegativeBase => transaction.base_amount = Decimal::from(-1),
        InvalidAmount::MissingQuoteAsset => transaction.quote_asset_id = None,
        InvalidAmount::MissingQuoteAmount => transaction.quote_amount = None,
        InvalidAmount::NegativeQuote => transaction.quote_amount = Some(Decimal::from(-1)),
        InvalidAmount::ZeroFee => transaction.fee_amount = Some(Decimal::ZERO),
        InvalidAmount::NegativeFee => transaction.fee_amount = Some(Decimal::from(-1)),
        InvalidAmount::MissingFeeAsset => transaction.fee_asset_id = None,
        InvalidAmount::MissingFeeAmount => transaction.fee_amount = None,
    }
}

fn assert_invalid_update_rejected(case: InvalidAmount, source: &str) {
    let mut fixture = Fixture::new();
    let valid = fixture.input();
    let previous = &mut fixture.store.data.transactions[0];
    invalidate(previous, case);
    previous.source = source.into();
    let before = serde_json::to_value(&fixture.store.data).unwrap();
    let mut input = fixture.input();
    input.notes = Some("Notes-only edit".into());
    assert!(
        update_transaction(&mut fixture.store, fixture.id, input).is_err(),
        "{case:?}"
    );
    assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), before);
    update_transaction(&mut fixture.store, fixture.id, valid).unwrap();
    assert_eq!(fixture.store.data.transactions[0].source, source);
}

#[test]
fn unchanged_invalid_amounts_require_correction_regardless_of_origin() {
    for source in ["manual", "historical.csv"] {
        for case in [
            InvalidAmount::ZeroBase,
            InvalidAmount::NegativeBase,
            InvalidAmount::MissingQuoteAsset,
            InvalidAmount::MissingQuoteAmount,
            InvalidAmount::NegativeQuote,
            InvalidAmount::ZeroFee,
            InvalidAmount::NegativeFee,
            InvalidAmount::MissingFeeAsset,
            InvalidAmount::MissingFeeAmount,
        ] {
            assert_invalid_update_rejected(case, source);
        }
    }
}

#[test]
fn invalid_historical_records_can_be_deleted_without_rewriting_preserved_raw_rows() {
    let mut fixture = Fixture::new();
    invalidate(
        &mut fixture.store.data.transactions[0],
        InvalidAmount::ZeroBase,
    );
    fixture
        .store
        .data
        .raw_rows
        .insert("old-row".into(), "historical payload".into());
    let rows = fixture.store.data.raw_rows.clone();
    delete_transaction(&mut fixture.store, fixture.id).unwrap();
    assert!(fixture.store.data.transactions.is_empty());
    assert_eq!(fixture.store.data.raw_rows, rows);
}
pub mod support;
