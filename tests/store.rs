use tuifolio::store::Store;

#[test]
fn store_lock_is_retained_by_clones_and_released_on_drop() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let store = Store::open(Some(path.clone())).unwrap();
    let clone = store.clone();
    assert!(Store::open(Some(path.clone())).is_err());
    drop(store);
    assert!(Store::open(Some(path.clone())).is_err());
    drop(clone);
    assert!(Store::open(Some(path)).is_ok());
}

#[test]
fn atomically_replaces_the_store_and_reopens_the_latest_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let mut store = Store::open(Some(path.clone())).unwrap();
    tuifolio::portfolios::create_portfolio(
        &mut store,
        tuifolio::portfolios::PortfolioInput {
            name: "First".into(),
        },
    )
    .unwrap();
    store.save().unwrap();
    tuifolio::portfolios::create_portfolio(
        &mut store,
        tuifolio::portfolios::PortfolioInput {
            name: "Second".into(),
        },
    )
    .unwrap();
    store.save().unwrap();
    drop(store);
    let reopened = Store::open(Some(path)).unwrap();
    assert_eq!(reopened.data.portfolios.len(), 2);
    assert_eq!(reopened.data.portfolios[1].name, "Second");
}

fn preservation_fixture() -> tuifolio::model::StoreData {
    use tuifolio::model::{Asset, AssetKind, AssetMetadataSource, StockSplit};
    let mut data = tuifolio::model::StoreData {
        next_id: 135,
        ..tuifolio::model::StoreData::default()
    };
    data.assets.push(Asset {
        id: 134,
        symbol: "GME".into(),
        name: "GameStop".into(),
        kind: AssetKind::Stock,
        yahoo_symbol: None,
        tradingview_symbol: None,
        valuation_currency: None,
        metadata_source: AssetMetadataSource::Automatic,
    });
    data.config.stock_splits.push(StockSplit {
        asset_id: 134,
        effective_date: "2022-07-22".into(),
        numerator: rust_decimal::Decimal::from(4),
        denominator: rust_decimal::Decimal::ONE,
    });
    data.raw_rows
        .insert("old-hash".into(), "original payload".into());
    data
}

#[test]
fn load_and_save_preserve_current_fields_and_asset_id_splits_without_provider_inference() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let before = serde_json::to_value(preservation_fixture()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&before).unwrap()).unwrap();
    let store = Store::open(Some(path.clone())).unwrap();
    assert!(store.data.assets[0].yahoo_quote_symbol().is_none());
    assert!(store.data.assets[0].tradingview_symbol.is_none());
    assert_eq!(store.data.config.stock_splits[0].asset_id, 134);
    store.save().unwrap();
    let after: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(path).unwrap()).unwrap();
    assert_eq!(after, before);
}

fn assert_store_rejected_without_rewriting(data: serde_json::Value, expected_error: &str) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let original = serde_json::to_vec_pretty(&data).unwrap();
    std::fs::write(&path, &original).unwrap();

    // Repeated failures must release the lock as well as leave the store untouched.
    for _ in 0..2 {
        let error = match Store::open(Some(path.clone())) {
            Ok(_) => panic!("unsupported datastore was accepted"),
            Err(error) => error,
        };
        let message = format!("{error:#}");
        assert!(message.contains(expected_error), "{message}");
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

#[test]
fn stores_reject_removed_transaction_posting_flags_without_rewriting() {
    use tuifolio::model::{LedgerEffect, Portfolio, Transaction, TransactionKind};

    let mut data = preservation_fixture();
    let portfolio_id = data.allocate_id();
    data.portfolios.push(Portfolio {
        id: portfolio_id,
        name: "Example".into(),
    });
    let id = data.allocate_id();
    data.transactions.push(Transaction {
        id,
        portfolio_id,
        timestamp: "2021-01-01T00:00:00Z".parse().unwrap(),
        kind: TransactionKind::Deposit,
        base_asset_id: 134,
        base_amount: rust_decimal::Decimal::from(10),
        quote_asset_id: None,
        quote_amount: None,
        quote_ledger_effect: LedgerEffect::Ignore,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
        source: "manual".into(),
        source_row_hash: format!("manual:{id}"),
    });
    for effect in ["Post", "Ignore"] {
        let mut legacy = serde_json::to_value(&data).unwrap();
        legacy["transactions"][0]["base_ledger_effect"] = effect.into();
        assert_store_rejected_without_rewriting(legacy, "unknown field `base_ledger_effect`");
    }
}

#[test]
fn stores_require_explicit_split_asset_ids_without_rewriting() {
    let mut data = serde_json::to_value(preservation_fixture()).unwrap();
    data["config"]["stock_splits"][0]
        .as_object_mut()
        .unwrap()
        .remove("asset_id");
    assert_store_rejected_without_rewriting(data, "missing field `asset_id`");
}

#[test]
fn stores_reject_symbol_based_splits_without_rewriting() {
    for include_asset_id in [false, true] {
        let mut data = serde_json::to_value(preservation_fixture()).unwrap();
        let split = data["config"]["stock_splits"][0].as_object_mut().unwrap();
        if !include_asset_id {
            split.remove("asset_id");
        }
        split.insert("symbol".into(), "GME".into());
        assert_store_rejected_without_rewriting(data, "unknown field `symbol`");
    }
}
