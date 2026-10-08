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
fn load_and_save_preserve_legacy_fields_and_converted_splits_without_provider_inference() {
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
