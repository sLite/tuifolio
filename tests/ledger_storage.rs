pub mod support;

use std::{fs, path::PathBuf};

use rust_decimal_macros::dec;
use serde_json::{Value, json};
use tuifolio::{
    accounting::build_report,
    model::{AssetKind, Id, LedgerEffect, StockSplit, StoreData, TransactionKind},
    price_sync::add_manual_price_for_asset_at,
    stock_splits::save_split,
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    data: StoreData,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.json");
        let mut store = Store::open(Some(path.clone())).unwrap();
        let cash = support::create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
        let stock = support::create_asset(&mut store, "ABC", "Stock", AssetKind::Stock);
        let portfolio = support::create_portfolio(&mut store, "Main");
        seed_transactions(&mut store, portfolio, stock, cash);
        seed_valuation(&mut store, stock);
        store.save().unwrap();
        Self {
            _directory: directory,
            path,
            data: store.data.clone(),
        }
    }

    fn saved(&self) -> Value {
        serde_json::from_slice(&fs::read(&self.path).unwrap()).unwrap()
    }

    fn write(&self, value: &Value) -> Vec<u8> {
        let bytes = serde_json::to_vec_pretty(value).unwrap();
        fs::write(&self.path, &bytes).unwrap();
        bytes
    }

    fn assert_cache_ignored(&self, cache: Value) {
        let mut value = self.saved();
        value["ledger_entries"] = cache;
        let original = self.write(&value);
        let store = Store::open(Some(self.path.clone())).unwrap();
        self.assert_accounting(&store.data);
        assert_eq!(fs::read(&self.path).unwrap(), original);
        store.save().unwrap();
        assert!(self.saved().get("ledger_entries").is_none());
        drop(store);
        self.assert_accounting(&Store::open(Some(self.path.clone())).unwrap().data);
    }

    fn assert_accounting(&self, actual: &StoreData) {
        assert_eq!(ledger(actual), ledger(&self.data));
        let expected = build_report(&self.data);
        let report = build_report(actual);
        assert_eq!(report.net_value, expected.net_value);
        assert_eq!(report.total_unrealized_pnl, expected.total_unrealized_pnl);
        assert_eq!(report.total_realized_pnl, expected.total_realized_pnl);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(&self.data).unwrap()
        );
    }
}

fn seed_valuation(store: &mut Store, stock: Id) {
    save_split(
        store,
        StockSplit {
            asset_id: stock,
            effective_date: "2021-01-01".into(),
            numerator: dec!(2),
            denominator: dec!(1),
        },
        None,
    )
    .unwrap();
    add_manual_price_for_asset_at(
        store,
        stock,
        dec!(200),
        "EUR",
        "2020-01-01T00:00:00Z".parse().unwrap(),
    )
    .unwrap();
}

fn seed_transactions(store: &mut Store, portfolio: Id, stock: Id, cash: Id) {
    let mut buy = transaction(portfolio, stock, cash);
    buy.fee_asset_id = Some(stock);
    buy.fee_amount = Some(dec!(1));
    add_manual_transaction(store, buy).unwrap();
    let mut sell = transaction(portfolio, stock, cash);
    sell.timestamp = "2022-01-01T00:00:00Z".parse().unwrap();
    sell.kind = TransactionKind::Sell;
    sell.base_amount = dec!(2);
    sell.quote_amount = Some(dec!(50));
    sell.fee_asset_id = Some(cash);
    sell.fee_amount = Some(dec!(1));
    add_manual_transaction(store, sell).unwrap();
}

fn transaction(portfolio: Id, stock: Id, cash: Id) -> ManualTransactionInput {
    ManualTransactionInput {
        portfolio_id: portfolio,
        timestamp: "2020-01-01T00:00:00Z".parse().unwrap(),
        kind: TransactionKind::Buy,
        base_asset_id: stock,
        base_amount: dec!(10),
        quote_asset_id: Some(cash),
        quote_amount: Some(dec!(100)),
        quote_ledger_effect: LedgerEffect::Post,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
    }
}

fn ledger(data: &StoreData) -> Value {
    serde_json::to_value(&data.ledger_entries).unwrap()
}

#[test]
fn saves_only_canonical_data_and_rebuilds_balances_fees_splits_and_pnl_on_reopen() {
    let fixture = Fixture::new();
    assert_eq!(fixture.data.ledger_entries.len(), 6);
    assert!(fixture.saved().get("ledger_entries").is_none());
    let reopened = Store::open(Some(fixture.path.clone())).unwrap();
    fixture.assert_accounting(&reopened.data);
    let before = ledger(&reopened.data);
    reopened.save().unwrap();
    assert_eq!(ledger(&reopened.data), before);
    assert!(fixture.saved().get("ledger_entries").is_none());
}

#[test]
fn json_roundtrip_does_not_serialize_or_restore_the_derived_cache() {
    let fixture = Fixture::new();
    let mut value = serde_json::to_value(&fixture.data).unwrap();
    assert!(value.get("ledger_entries").is_none());
    value["ledger_entries"] = ledger(&fixture.data);
    let mut decoded: StoreData = serde_json::from_value(value).unwrap();
    assert!(decoded.ledger_entries.is_empty());
    tuifolio::ledger::rebuild_ledger(&mut decoded).unwrap();
    fixture.assert_accounting(&decoded);
}

#[test]
fn ignores_any_existing_json_cache_without_rewriting_until_a_successful_save() {
    let fixture = Fixture::new();
    let mut stale = ledger(&fixture.data);
    stale[0]["asset_id"] = json!(999);
    stale[0]["portfolio_id"] = json!(999);
    stale[0]["transaction_id"] = json!(999);
    stale[0]["quantity_delta"] = json!("invalid decimal");
    for cache in [
        stale,
        json!([]),
        json!(null),
        json!(false),
        json!("not a ledger"),
        json!({}),
    ] {
        fixture.assert_cache_ignored(cache);
    }
}

#[test]
fn ignored_cache_cannot_hide_invalid_transactions_or_split_configuration() {
    let fixture = Fixture::new();
    for defect in ["transaction", "split"] {
        let mut value = fixture.saved();
        value["ledger_entries"] = ledger(&fixture.data);
        if defect == "transaction" {
            value["transactions"][0]["base_asset_id"] = json!(999);
        } else {
            value["config"]["stock_splits"][0]["denominator"] = json!("0");
        }
        let original = fixture.write(&value);
        for _ in 0..2 {
            assert!(Store::open(Some(fixture.path.clone())).is_err());
            assert_eq!(fs::read(&fixture.path).unwrap(), original);
        }
    }
    fixture.write(&serde_json::to_value(&fixture.data).unwrap());
    assert!(Store::open(Some(fixture.path.clone())).is_ok());
}

#[test]
fn failed_save_retains_existing_json_and_the_rebuilt_in_memory_cache() {
    let fixture = Fixture::new();
    let mut value = fixture.saved();
    value["ledger_entries"] = json!("discard this cache");
    let original = fixture.write(&value);
    let mut store = Store::open(Some(fixture.path.clone())).unwrap();
    let rebuilt = ledger(&store.data);
    store.data.next_id = 0;
    assert!(store.save().is_err());
    assert_eq!(fs::read(&fixture.path).unwrap(), original);
    assert_eq!(ledger(&store.data), rebuilt);
}
