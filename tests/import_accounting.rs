use std::path::Path;

use rust_decimal::Decimal;
use tuifolio::{
    accounting::build_report,
    importer::import_delta_dir,
    ledger::rebuild_ledger,
    model::{AssetKind, LedgerEffect, Transaction, TransactionKind, split_delta_asset},
    price_sync::add_manual_price,
    store::Store,
};

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
    assert_eq!(store.data.transactions.len(), 148);
    assert!(!store.data.ledger_entries.is_empty());
}

#[test]
fn imports_house_as_asset_and_liability() {
    let mut store = temp_store();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("delta-exports");
    import_delta_dir(&mut store, &dir).unwrap();

    let has_liability = store
        .data
        .assets
        .iter()
        .any(|asset| asset.kind == AssetKind::Liability);
    let report = build_report(&store.data);

    assert!(has_liability);
    assert!(
        report
            .negative_balances
            .iter()
            .any(|row| row.portfolio == "House")
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
        matches!(row.portfolio.as_str(), "Crypto" | "Stocks" | "Metal")
            && matches!(row.kind, AssetKind::Fiat)
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

fn temp_store() -> Store {
    let path = std::env::temp_dir().join(format!("tuifolio-test-{}.json", std::process::id()));
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    Store::open(Some(path)).unwrap()
}
