use std::path::Path;

use rust_decimal::Decimal;
use tuifolio::{
    accounting::build_report,
    importer::{import_delta_dir, import_delta_file, split_delta_asset},
    ledger::rebuild_ledger,
    model::{AssetKind, LedgerEffect, StockSplit, Transaction, TransactionKind},
    price_sync::add_manual_price,
    store::Store,
};

#[test]
fn imports_tradingview_symbols_for_known_assets() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();

    assert_tradingview_symbol(&store, "BTC", "CRYPTO:BTCUSD");
    assert_tradingview_symbol(&store, "GOLD", "OANDA:XAUUSD");
    assert_tradingview_symbol(&store, "GME", "NYSE:GME");
    assert_tradingview_symbol(&store, "BYND", "NASDAQ:BYND");
    assert_tradingview_symbol(&store, "INTC", "NASDAQ:INTC");
    assert_tradingview_symbol(&store, "VWCE.DE", "XETR:VWCE");
    assert_tradingview_symbol(&store, "EI4.F", "FWB:EI4");
    assert_tradingview_symbol(&store, "639.DE", "XETR:639");
}

#[test]
fn store_open_migrates_legacy_tradingview_symbols() {
    let path = temp_store_path();
    let legacy_store = serde_json::json!({
        "config": {
            "base_currencies": ["EUR", "USD", "BTC", "ETH"],
            "default_base_currency": "EUR",
            "selected_base_currency": "EUR",
            "stock_splits": []
        },
        "next_id": 3,
        "portfolios": [],
        "assets": [
            {
                "id": 1,
                "symbol": "GOLD",
                "name": "Gold",
                "kind": "Commodity",
                "yahoo_symbol": "GC=F",
                "valuation_currency": null
            },
            {
                "id": 2,
                "symbol": "GME",
                "name": "GameStop Corp",
                "kind": "Stock",
                "yahoo_symbol": "GME",
                "valuation_currency": null
            }
        ],
        "transactions": [],
        "ledger_entries": [],
        "prices": [],
        "raw_rows": {}
    });
    std::fs::write(&path, serde_json::to_string_pretty(&legacy_store).unwrap()).unwrap();

    let store = Store::open(Some(path)).unwrap();

    assert_tradingview_symbol(&store, "GOLD", "OANDA:XAUUSD");
    assert_tradingview_symbol(&store, "GME", "NYSE:GME");
}

#[test]
fn splits_delta_asset_labels() {
    assert_eq!(
        split_delta_asset("BTC (Bitcoin)"),
        ("BTC".into(), "Bitcoin".into())
    );
    assert_eq!(
        split_delta_asset("DOT* (Polkadot)"),
        ("DOT".into(), "Polkadot".into())
    );
    assert_eq!(split_delta_asset("EUR"), ("EUR".into(), "EUR".into()));
}

#[test]
fn imports_all_delta_rows_idempotently() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");

    let first = import_delta_dir(&mut store, &dir).unwrap();
    let second = import_delta_dir(&mut store, &dir).unwrap();

    assert_eq!(first.imported, 148);
    assert_eq!(first.skipped, 0);
    assert_eq!(second.imported, 0);
    assert_eq!(second.skipped, 148);
    assert_eq!(store.data.transactions.len(), 150);
    assert!(!store.data.ledger_entries.is_empty());
}

#[test]
fn imports_house_as_asset_and_liability() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();

    let property = store
        .data
        .assets
        .iter()
        .find(|asset| asset.symbol == "PROPERTY" && asset.kind == AssetKind::Property)
        .unwrap();
    let mortgage = store
        .data
        .assets
        .iter()
        .find(|asset| asset.symbol == "MORTGAGE" && asset.kind == AssetKind::Liability)
        .unwrap();
    let property_transactions = store
        .data
        .transactions
        .iter()
        .filter(|transaction| transaction.base_asset_id == property.id)
        .collect::<Vec<_>>();
    let mortgage_transactions = store
        .data
        .transactions
        .iter()
        .filter(|transaction| transaction.base_asset_id == mortgage.id)
        .collect::<Vec<_>>();
    let report = build_report(&store.data);
    let house = report
        .portfolios
        .iter()
        .find(|portfolio| portfolio.name == "Hintersdorf")
        .unwrap();

    assert_eq!(property.name, "Grundstück");
    assert_eq!(property.valuation_currency.as_deref(), Some("EUR"));
    assert_eq!(property_transactions.len(), 2);
    assert_eq!(
        property_transactions[0].kind,
        TransactionKind::AssetIncrease
    );
    assert_eq!(
        property_transactions[0].base_amount,
        Decimal::new(255000, 0)
    );
    assert_eq!(
        property_transactions[1].kind,
        TransactionKind::AssetIncrease
    );
    assert_eq!(
        property_transactions[1].base_amount,
        Decimal::new(345000, 0)
    );
    assert_eq!(mortgage.name, "Kredit");
    assert_eq!(mortgage.valuation_currency.as_deref(), Some("EUR"));
    assert_eq!(mortgage_transactions.len(), 2);
    assert_eq!(
        mortgage_transactions[0].kind,
        TransactionKind::LiabilityIncrease
    );
    assert_eq!(
        mortgage_transactions[0].base_amount,
        Decimal::new(255000, 0)
    );
    assert_eq!(
        mortgage_transactions[1].kind,
        TransactionKind::LiabilityDecrease
    );
    assert_eq!(
        mortgage_transactions[1].base_amount,
        Decimal::new(145000, 0)
    );
    assert_eq!(house.assets, Decimal::new(600000, 0));
    assert_eq!(house.liabilities, Decimal::new(110000, 0));
    assert_eq!(house.net_value, Decimal::new(490000, 0));
    assert!(
        !report
            .negative_balances
            .iter()
            .any(|row| row.portfolio == "Hintersdorf")
    );
}

#[test]
fn import_does_not_create_artificial_negative_quote_cash() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();
    let report = build_report(&store.data);

    assert!(
        !report
            .negative_balances
            .iter()
            .any(|row| row.portfolio == "Stocks" && matches!(row.symbol.as_str(), "EUR" | "USD"))
    );
    assert!(
        !report
            .negative_balances
            .iter()
            .any(|row| row.portfolio == "Metal" && row.symbol == "USD")
    );
}

#[test]
fn sync_base_rows_do_not_double_count_quote_asset() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();

    let btc_id = store
        .data
        .assets
        .iter()
        .find(|asset| asset.symbol == "BTC")
        .unwrap()
        .id;
    let crypto_id = store
        .data
        .portfolios
        .iter()
        .find(|portfolio| portfolio.name == "Crypto")
        .unwrap()
        .id;
    let balance = store
        .data
        .ledger_entries
        .iter()
        .filter(|entry| entry.portfolio_id == crypto_id && entry.asset_id == btc_id)
        .map(|entry| entry.quantity_delta)
        .sum::<Decimal>();

    assert_eq!(balance, Decimal::new(24050386, 8));
}

#[test]
fn sync_base_companion_rows_do_not_create_synthetic_quote_cash() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();

    let usd_id = store
        .data
        .assets
        .iter()
        .find(|asset| asset.symbol == "USD" && asset.kind == AssetKind::Fiat)
        .unwrap()
        .id;
    let crypto_id = store
        .data
        .portfolios
        .iter()
        .find(|portfolio| portfolio.name == "Crypto")
        .unwrap()
        .id;
    let balance = store
        .data
        .ledger_entries
        .iter()
        .filter(|entry| entry.portfolio_id == crypto_id && entry.asset_id == usd_id)
        .map(|entry| entry.quantity_delta)
        .sum::<Decimal>();

    assert_eq!(balance, Decimal::ZERO);
}

#[test]
fn delta_import_ignores_fiat_holdings_outside_fiat_and_house() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();

    let report = build_report(&store.data);

    assert!(!report.holdings.iter().any(|row| {
        matches!(row.portfolio.as_str(), "Crypto" | "Metal") && matches!(row.kind, AssetKind::Fiat)
    }));
}

#[test]
fn stock_splits_adjust_imported_share_balances() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();

    let gme_id = store
        .data
        .assets
        .iter()
        .find(|asset| asset.symbol == "GME")
        .unwrap()
        .id;
    let stocks_id = store
        .data
        .portfolios
        .iter()
        .find(|portfolio| portfolio.name == "Stocks")
        .unwrap()
        .id;
    let balance = store
        .data
        .ledger_entries
        .iter()
        .filter(|entry| entry.portfolio_id == stocks_id && entry.asset_id == gme_id)
        .map(|entry| entry.quantity_delta)
        .sum::<Decimal>();

    assert_eq!(balance, Decimal::new(81, 0));
}

#[test]
fn stock_splits_adjust_single_file_imports() {
    let mut store = temp_store();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("delta-exports")
        .join("delta_Stocks_18122025.csv");
    import_delta_file(&mut store, &path).unwrap();

    let gme_id = store
        .data
        .assets
        .iter()
        .find(|asset| asset.symbol == "GME")
        .unwrap()
        .id;
    let stocks_id = store
        .data
        .portfolios
        .iter()
        .find(|portfolio| portfolio.name == "Stocks")
        .unwrap()
        .id;
    let balance = store
        .data
        .ledger_entries
        .iter()
        .filter(|entry| entry.portfolio_id == stocks_id && entry.asset_id == gme_id)
        .map(|entry| entry.quantity_delta)
        .sum::<Decimal>();

    assert_eq!(balance, Decimal::new(81, 0));
}

#[test]
fn store_open_migrates_legacy_symbol_stock_splits() {
    let path = std::env::temp_dir().join(format!(
        "tuifolio-legacy-split-test-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        r#"{
  "config": {
    "base_currencies": ["EUR"],
    "default_base_currency": "EUR",
    "selected_base_currency": "EUR",
    "stock_splits": [{
      "symbol": "GME",
      "effective_date": "2022-07-22",
      "numerator": "4",
      "denominator": "1"
    }]
  },
  "next_id": 2,
  "portfolios": [],
  "assets": [{
    "id": 1,
    "symbol": "GME",
    "name": "GameStop",
    "kind": "Stock"
  }],
  "transactions": [],
  "ledger_entries": [],
  "prices": [],
  "raw_rows": {}
}"#,
    )
    .unwrap();

    let store = Store::open(Some(path.clone())).unwrap();
    let _ = std::fs::remove_file(path);

    assert_eq!(store.data.config.stock_splits[0].asset_id, 1);
}

#[test]
fn imported_stocks_eur_deposit_posts_to_cash_holding() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();

    let report = build_report(&store.data);

    assert!(report.holdings.iter().any(|row| {
        row.portfolio == "Stocks"
            && row.symbol == "EUR"
            && row.kind == AssetKind::Fiat
            && row.quantity == Decimal::new(4946, 0)
    }));
}

#[test]
fn manual_prices_enable_market_valuation() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();
    add_manual_price(&mut store, "GOLD", Decimal::new(3000, 0), "EUR").unwrap();

    let report = build_report(&store.data);

    assert!(
        report
            .holdings
            .iter()
            .any(|row| row.symbol == "GOLD" && row.value.is_some())
    );
}

#[test]
fn manual_cost_only_quote_amount_does_not_create_cash_holding() {
    let mut store = temp_store();
    store.data.config.selected_base_currency = "USD".into();
    let portfolio_id = store.portfolio_id("Crypto");
    let btc_id = store.asset_id("BTC", "Bitcoin", AssetKind::Crypto);
    let usd_id = store.asset_id("USD", "USD", AssetKind::Fiat);
    let transaction_id = store.data.allocate_id();

    store.data.transactions.push(Transaction {
        id: transaction_id,
        portfolio_id,
        timestamp: chrono::Utc::now(),
        kind: TransactionKind::Buy,
        base_asset_id: btc_id,
        base_amount: Decimal::ONE,
        base_ledger_effect: LedgerEffect::Post,
        quote_asset_id: Some(usd_id),
        quote_amount: Some(Decimal::new(10_000, 0)),
        quote_ledger_effect: LedgerEffect::Ignore,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
        source: "manual".into(),
        source_row_hash: String::new(),
    });
    rebuild_ledger(&mut store.data).unwrap();
    add_manual_price(&mut store, "BTC", Decimal::new(50_000, 0), "USD").unwrap();

    let report = build_report(&store.data);

    assert!(report.holdings.iter().any(|row| {
        row.symbol == "BTC"
            && row.quantity == Decimal::ONE
            && row.net_invested == Some(Decimal::new(10_000, 0))
    }));
    assert!(
        !report
            .holdings
            .iter()
            .any(|row| row.symbol == "USD" && row.portfolio == "Crypto")
    );
}

#[test]
fn ledger_posts_quote_amounts_even_when_cash_goes_negative() {
    let mut store = temp_store();
    let portfolio_id = store.portfolio_id("Manual");
    let stock_id = store.asset_id("ABC", "ABC", AssetKind::Stock);
    let usd_id = store.asset_id("USD", "USD", AssetKind::Fiat);
    let transaction_id = store.data.allocate_id();

    store.data.transactions.push(Transaction {
        id: transaction_id,
        portfolio_id,
        timestamp: chrono::Utc::now(),
        kind: TransactionKind::Buy,
        base_asset_id: stock_id,
        base_amount: Decimal::ONE,
        base_ledger_effect: LedgerEffect::Post,
        quote_asset_id: Some(usd_id),
        quote_amount: Some(Decimal::new(100, 0)),
        quote_ledger_effect: LedgerEffect::Post,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
        source: "manual".into(),
        source_row_hash: String::new(),
    });
    rebuild_ledger(&mut store.data).unwrap();

    let usd_balance = store
        .data
        .ledger_entries
        .iter()
        .filter(|entry| entry.portfolio_id == portfolio_id && entry.asset_id == usd_id)
        .map(|entry| entry.quantity_delta)
        .sum::<Decimal>();

    assert_eq!(usd_balance, Decimal::new(-100, 0));
}

#[test]
fn explicit_split_events_adjust_only_the_target_asset() {
    let mut store = temp_store();
    let portfolio_id = store.portfolio_id("Manual");
    let split_asset_id = store.asset_id("ABC", "ABC", AssetKind::Stock);
    let other_asset_id = store.asset_id("XYZ", "XYZ", AssetKind::Stock);
    store.data.config.stock_splits.push(StockSplit {
        asset_id: split_asset_id,
        legacy_symbol: None,
        effective_date: "2024-01-01".into(),
        numerator: Decimal::new(2, 0),
        denominator: Decimal::ONE,
    });
    for asset_id in [split_asset_id, other_asset_id] {
        let transaction_id = store.data.allocate_id();
        store.data.transactions.push(Transaction {
            id: transaction_id,
            portfolio_id,
            timestamp: chrono::DateTime::parse_from_rfc3339("2023-01-01T00:00:00.000Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            kind: TransactionKind::Deposit,
            base_asset_id: asset_id,
            base_amount: Decimal::new(10, 0),
            base_ledger_effect: LedgerEffect::Post,
            quote_asset_id: None,
            quote_amount: None,
            quote_ledger_effect: LedgerEffect::Post,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: None,
            source: "manual".into(),
            source_row_hash: format!("manual:{asset_id}"),
        });
    }
    rebuild_ledger(&mut store.data).unwrap();

    let balance = |asset_id| {
        store
            .data
            .ledger_entries
            .iter()
            .filter(|entry| entry.portfolio_id == portfolio_id && entry.asset_id == asset_id)
            .map(|entry| entry.quantity_delta)
            .sum::<Decimal>()
    };

    assert_eq!(balance(split_asset_id), Decimal::new(20, 0));
    assert_eq!(balance(other_asset_id), Decimal::new(10, 0));
}

#[test]
fn ledger_rebuild_fails_on_missing_posted_asset() {
    let mut store = temp_store();
    let portfolio_id = store.portfolio_id("Manual");
    let transaction_id = store.data.allocate_id();

    store.data.transactions.push(Transaction {
        id: transaction_id,
        portfolio_id,
        timestamp: chrono::Utc::now(),
        kind: TransactionKind::Deposit,
        base_asset_id: 999,
        base_amount: Decimal::ONE,
        base_ledger_effect: LedgerEffect::Post,
        quote_asset_id: None,
        quote_amount: None,
        quote_ledger_effect: LedgerEffect::Post,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
        source: "manual".into(),
        source_row_hash: String::new(),
    });

    assert!(rebuild_ledger(&mut store.data).is_err());
}

fn temp_store() -> Store {
    let path = temp_store_path();
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    Store::open(Some(path)).unwrap()
}

fn temp_store_path() -> std::path::PathBuf {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "tuifolio-test-{}-{suffix}.json",
        std::process::id()
    ))
}

fn assert_tradingview_symbol(store: &Store, symbol: &str, tradingview_symbol: &str) {
    let asset = store
        .data
        .assets
        .iter()
        .find(|asset| asset.symbol == symbol)
        .unwrap_or_else(|| panic!("missing asset {symbol}"));
    assert_eq!(
        asset.tradingview_symbol.as_deref(),
        Some(tradingview_symbol)
    );
}
