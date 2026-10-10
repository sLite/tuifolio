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

#[cfg(unix)]
#[test]
fn symlink_file_and_target_share_lock_and_save_without_forking() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("store.json");
    let alias = directory.path().join("alias.json");
    let store = Store::open(Some(target.clone())).unwrap();
    store.save().unwrap();
    drop(store);
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let mut store = Store::open(Some(alias.clone())).unwrap();
    assert_eq!(store.path(), &target);
    assert!(Store::open(Some(target.clone())).is_err());
    tuifolio::portfolios::create_portfolio(
        &mut store,
        tuifolio::portfolios::PortfolioInput {
            name: "Shared".into(),
        },
    )
    .unwrap();
    store.save().unwrap();
    assert!(
        std::fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    drop(store);
    let reopened = Store::open(Some(target.clone())).unwrap();
    assert_eq!(reopened.data.portfolios[0].name, "Shared");
    assert!(Store::open(Some(alias)).is_err());
}

#[cfg(unix)]
#[test]
fn new_paths_under_symlinked_parents_share_the_same_lock() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("data");
    let alias = directory.path().join("alias");
    std::fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let store = Store::open(Some(alias.join("new.json"))).unwrap();
    assert_eq!(store.path(), &target.join("new.json"));
    assert!(Store::open(Some(target.join("new.json"))).is_err());
    store.save().unwrap();
    assert!(
        std::fs::symlink_metadata(alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn dangling_store_symlinks_are_rejected_without_replacing_them() {
    let directory = tempfile::tempdir().unwrap();
    let alias = directory.path().join("alias.json");
    std::os::unix::fs::symlink(directory.path().join("missing.json"), &alias).unwrap();
    assert!(Store::open(Some(alias.clone())).is_err());
    assert!(
        std::fs::symlink_metadata(alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!directory.path().join("missing.json").exists());
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
fn structural_defects_are_rejected_without_rewriting_or_retaining_locks() {
    let original = serde_json::to_value(preservation_fixture()).unwrap();
    let mut duplicate = original.clone();
    let asset = duplicate["assets"][0].clone();
    duplicate["assets"].as_array_mut().unwrap().push(asset);
    assert_store_rejected_without_rewriting(duplicate, "duplicate datastore ID");
    let mut allocator = original.clone();
    allocator["next_id"] = 134.into();
    assert_store_rejected_without_rewriting(allocator, "next_id must exceed");
    let mut split = original.clone();
    split["config"]["stock_splits"][0]["asset_id"] = 999.into();
    assert_store_rejected_without_rewriting(split, "split references missing asset");
    let mut split = original.clone();
    split["config"]["stock_splits"][0]["denominator"] = "0".into();
    assert_store_rejected_without_rewriting(split, "must be positive");
    let mut price = original.clone();
    price["prices"] = serde_json::json!([{
        "asset_id":999, "timestamp":"2020-01-01T00:00:00Z",
        "price":"1", "currency":"EUR", "source":"fixture"
    }]);
    assert_store_rejected_without_rewriting(price, "price references missing asset");
    let mut config = original.clone();
    config["config"]["selected_base_currency"] = "UNKNOWN".into();
    assert_store_rejected_without_rewriting(config, "is not configured");
    let mut collision = original;
    collision["portfolios"] = serde_json::json!([{"id":134,"name":"Collision"}]);
    assert_store_rejected_without_rewriting(collision, "duplicate datastore ID");
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
