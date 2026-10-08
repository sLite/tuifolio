use rust_decimal::Decimal;
use tuifolio::{
    importer::import_delta_dir,
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
        let base = store.asset_id("BTC", "Bitcoin", AssetKind::Crypto);
        let quote = store.asset_id("EUR", "Euro", AssetKind::Fiat);
        let fee = store.asset_id("USD", "US dollar", AssetKind::Fiat);
        let other_asset = store.asset_id("ETH", "Ethereum", AssetKind::Crypto);
        let portfolio = store.portfolio_id("Main");
        let other_portfolio = store.portfolio_id("Other");
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
        base_ledger_effect: LedgerEffect::Post,
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
fn imported_transactions_can_be_updated_without_changing_unchanged_legacy_values() {
    let mut fixture = imported_fixture();
    let before = serde_json::to_value(&fixture.store.data).unwrap();
    let originals = fixture.store.data.transactions.clone();
    for transaction in originals {
        update_transaction(
            &mut fixture.store,
            transaction.id,
            ManualTransactionInput::from(&transaction),
        )
        .unwrap();
    }
    assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), before);
}

#[test]
fn reimport_does_not_undo_edits_or_resurrect_deleted_transactions() {
    let mut fixture = imported_fixture();
    let exports = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    let original = fixture.store.data.transactions[0].clone();
    let deleted = fixture.store.data.transactions[1].id;
    let mut input = ManualTransactionInput::from(&original);
    input.notes = Some("Corrected import".into());
    update_transaction(&mut fixture.store, original.id, input).unwrap();
    delete_transaction(&mut fixture.store, deleted).unwrap();
    let raw_rows = fixture.store.data.raw_rows.clone();
    let summary = import_delta_dir(&mut fixture.store, &exports).unwrap();
    assert_eq!(summary.imported, 0);
    assert_eq!(fixture.store.data.raw_rows, raw_rows);
    assert!(
        !fixture
            .store
            .data
            .transactions
            .iter()
            .any(|transaction| transaction.id == deleted)
    );
    let updated = fixture
        .store
        .data
        .transactions
        .iter()
        .find(|transaction| transaction.id == original.id)
        .unwrap();
    assert_eq!(updated.notes.as_deref(), Some("Corrected import"));
}

fn imported_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.store.reset();
    let exports = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut fixture.store, &exports).unwrap();
    fixture
}

#[test]
fn ignored_asset_and_quote_postings_remain_ignored_until_explicitly_changed() {
    let mut fixture = Fixture::new();
    fixture.store.data.transactions[0].base_ledger_effect = LedgerEffect::Ignore;
    fixture.store.data.transactions[0].quote_ledger_effect = LedgerEffect::Ignore;
    rebuild_ledger(&mut fixture.store.data).unwrap();
    let mut input = fixture.input();
    input.notes = Some("Changed notes".into());
    update_transaction(&mut fixture.store, fixture.id, input).unwrap();
    assert_eq!(fixture.store.data.ledger_entries.len(), 1);
    let mut input = fixture.input();
    input.base_ledger_effect = LedgerEffect::Post;
    update_transaction(&mut fixture.store, fixture.id, input).unwrap();
    assert_eq!(fixture.store.data.ledger_entries.len(), 2);
}
