use std::process::Command;
use tuifolio::model::{Asset, AssetKind, AssetMetadataSource, StoreData};

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
        metadata_source: AssetMetadataSource::User,
    });
    data
}

#[test]
fn cli_rejects_stock_or_unknown_base_without_saving_and_normalizes_valid_base() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let original = serde_json::to_vec(&fixture()).unwrap();
    std::fs::write(&path, &original).unwrap();
    for currency in ["AAPL", "NOTACURRENCY", ""] {
        let output = Command::new(env!("CARGO_BIN_EXE_tuifolio"))
            .args(["--store", path.to_str().unwrap(), "base", currency])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_tuifolio"))
        .args(["--store", path.to_str().unwrap(), "base", " usd "])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: StoreData = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data.config.selected_base_currency, "USD");
    assert_eq!(data.config.base_currencies, ["EUR", "USD", "BTC", "ETH"]);
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
