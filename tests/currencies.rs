use tuifolio::model::{Asset, AssetKind, StoreData};

fn fixture() -> StoreData {
    let mut data = StoreData {
        next_id: 2,
        ..StoreData::default()
    };
    data.assets.push(Asset {
        id: 1,
        symbol: "AAPL".into(),
        name: "Stock".into(),
        kind: AssetKind::Stock,
        yahoo_symbol: None,
        tradingview_symbol: None,
        valuation_currency: None,
    });
    data
}

#[test]
fn persisted_stock_reporting_configuration_is_rejected_without_rewriting() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let mut data = fixture();
    data.config.base_currencies.push("AAPL".into());
    data.config.selected_base_currency = "AAPL".into();
    let original = serde_json::to_vec(&data).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(tuifolio::store::Store::open(Some(path.clone())).is_err());
    assert_eq!(std::fs::read(path).unwrap(), original);
}
