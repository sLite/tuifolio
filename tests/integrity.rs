use tuifolio::{integrity::validate_store, model::*};

fn fixture() -> StoreData {
    let mut data = StoreData {
        next_id: 4,
        ..StoreData::default()
    };
    data.portfolios.push(Portfolio {
        id: 1,
        name: "Main".into(),
    });
    data.assets.push(Asset {
        id: 2,
        symbol: "EUR".into(),
        name: "Euro".into(),
        kind: AssetKind::Fiat,
        yahoo_symbol: None,
        tradingview_symbol: None,
        valuation_currency: None,
        metadata_source: AssetMetadataSource::User,
    });
    data.transactions.push(Transaction {
        id: 3,
        portfolio_id: 1,
        timestamp: "2020-01-01T00:00:00Z".parse().unwrap(),
        kind: TransactionKind::Deposit,
        base_asset_id: 2,
        base_amount: 10.into(),
        quote_asset_id: Some(2),
        quote_amount: None,
        quote_ledger_effect: LedgerEffect::Ignore,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
        source: "historical".into(),
        source_row_hash: "historical:3".into(),
    });
    tuifolio::ledger::rebuild_ledger(&mut data).unwrap();
    data
}

#[test]
fn structural_validation_preserves_ignored_historical_deposit_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let original = serde_json::to_vec(&fixture()).unwrap();
    std::fs::write(&path, &original).unwrap();
    let store = tuifolio::store::Store::open(Some(path.clone())).unwrap();
    assert_eq!(store.data.transactions[0].quote_asset_id, Some(2));
    assert_eq!(store.data.transactions[0].quote_amount, None);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    store.save().unwrap();
    let saved: StoreData = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(saved).unwrap(),
        serde_json::to_value(fixture()).unwrap()
    );
}

#[test]
fn all_structural_references_are_checked_even_when_not_posted() {
    for field in [
        "portfolio",
        "base",
        "quote",
        "fee",
        "ledger_asset",
        "ledger_portfolio",
        "ledger_transaction",
    ] {
        let mut data = fixture();
        match field {
            "portfolio" => data.transactions[0].portfolio_id = 999,
            "base" => data.transactions[0].base_asset_id = 999,
            "quote" => data.transactions[0].quote_asset_id = Some(999),
            "fee" => data.transactions[0].fee_asset_id = Some(999),
            "ledger_asset" => data.ledger_entries[0].asset_id = 999,
            "ledger_portfolio" => data.ledger_entries[0].portfolio_id = 999,
            "ledger_transaction" => data.ledger_entries[0].transaction_id = 999,
            _ => unreachable!(),
        }
        assert!(
            validate_store(&data)
                .unwrap_err()
                .to_string()
                .contains("missing"),
            "{field}"
        );
    }
}

#[test]
fn invalid_store_cannot_load_rewrite_or_hold_its_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let mut data = fixture();
    data.transactions[0].portfolio_id = 999;
    let original = serde_json::to_vec(&data).unwrap();
    std::fs::write(&path, &original).unwrap();
    for _ in 0..2 {
        assert!(tuifolio::store::Store::open(Some(path.clone())).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    std::fs::write(&path, serde_json::to_vec(&fixture()).unwrap()).unwrap();
    assert!(tuifolio::store::Store::open(Some(path)).is_ok());
}
