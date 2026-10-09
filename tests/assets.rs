use chrono::Utc;
use rust_decimal_macros::dec;
use tuifolio::{
    accounting::build_report,
    assets::{AssetInput, save_asset},
    model::{Asset, AssetKind, AssetMetadataSource, Id, LedgerEffect, Price, TransactionKind},
    price_sync::{PriceBatch, SyncSummary, fetch_prices, merge_price_batch},
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

struct Fixture {
    directory: tempfile::TempDir,
    store: Store,
    id: Id,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(Some(directory.path().join("store.json"))).unwrap();
        save_asset(
            &mut store,
            None,
            AssetInput {
                symbol: "AAPL".into(),
                name: "Apple".into(),
                kind: AssetKind::Stock,
                yahoo_symbol: Some("AAPL".into()),
                tradingview_symbol: None,
                valuation_currency: None,
            },
        )
        .unwrap();
        support::create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
        support::create_portfolio(&mut store, "Main");
        let input = stock_transaction(&store);
        let result = add_manual_transaction(&mut store, input).unwrap();
        record_yahoo_price(&mut store, result.asset_id);
        Self {
            directory,
            store,
            id: result.asset_id,
        }
    }

    fn asset(&self) -> &Asset {
        self.store
            .data
            .assets
            .iter()
            .find(|asset| asset.id == self.id)
            .unwrap()
    }

    fn input(&self) -> AssetInput {
        let asset = self.asset();
        AssetInput {
            symbol: asset.symbol.clone(),
            name: asset.name.clone(),
            kind: asset.kind,
            yahoo_symbol: asset.yahoo_symbol.clone(),
            tradingview_symbol: asset.tradingview_symbol.clone(),
            valuation_currency: asset.valuation_currency.clone(),
        }
    }

    fn reopen(self) -> Self {
        let path = self.store.path().clone();
        drop(self.store);
        Self {
            directory: self.directory,
            id: self.id,
            store: Store::open(Some(path)).unwrap(),
        }
    }
}

fn record_yahoo_price(store: &mut Store, asset_id: Id) {
    store.data.prices.push(Price {
        asset_id,
        timestamp: Utc::now(),
        price: dec!(100),
        currency: "EUR".into(),
        source: "yahoo".into(),
    });
}

fn stock_transaction(store: &Store) -> ManualTransactionInput {
    ManualTransactionInput {
        portfolio_id: store
            .data
            .portfolios
            .iter()
            .find(|portfolio| portfolio.name == "Main")
            .unwrap()
            .id,
        timestamp: Utc::now(),
        kind: TransactionKind::Buy,
        base_asset_id: store.asset_by_symbol("AAPL").unwrap().id,
        base_amount: dec!(2),
        quote_asset_id: Some(store.asset_by_symbol("EUR").unwrap().id),
        quote_amount: Some(dec!(150)),
        quote_ledger_effect: LedgerEffect::Ignore,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
    }
}

#[test]
fn editing_display_names_preserves_asset_identity_transactions_prices_and_balances() {
    let mut fixture = Fixture::new();
    let original = serde_json::to_value(&fixture.store.data.ledger_entries).unwrap();
    let mut input = fixture.input();
    input.name = "Apple Inc.".into();
    save_asset(&mut fixture.store, Some(fixture.id), input).unwrap();
    assert_eq!(fixture.asset().symbol, "AAPL");
    assert_eq!(fixture.asset().name, "Apple Inc.");
    assert_eq!(fixture.store.data.transactions[0].base_asset_id, fixture.id);
    assert_eq!(fixture.store.data.prices[0].asset_id, fixture.id);
    assert_eq!(
        serde_json::to_value(&fixture.store.data.ledger_entries).unwrap(),
        original
    );
    let report = build_report(&fixture.store.data);
    let holding = report
        .holdings
        .iter()
        .find(|holding| holding.symbol == "AAPL")
        .unwrap();
    assert_eq!(holding.quantity, dec!(2));
    assert_eq!(holding.value, Some(dec!(200)));
}

#[test]
fn cleared_provider_settings_survive_reopen_transactions_and_price_refresh() {
    let mut fixture = Fixture::new();
    let mut input = fixture.input();
    input.yahoo_symbol = None;
    input.tradingview_symbol = None;
    save_asset(&mut fixture.store, Some(fixture.id), input).unwrap();
    fixture.store.save().unwrap();
    let mut fixture = fixture.reopen();
    let input = stock_transaction(&fixture.store);
    add_manual_transaction(&mut fixture.store, input).unwrap();
    assert_eq!(fixture.asset().metadata_source, AssetMetadataSource::User);
    assert!(fixture.asset().yahoo_symbol.is_none());
    assert!(fixture.asset().tradingview_symbol.is_none());
    let batch = fetch_prices(&fixture.store.data).unwrap();
    assert!(
        !batch
            .prices
            .iter()
            .any(|price| price.asset_id == fixture.id)
    );
    assert_eq!(batch.summary.unsupported, 1);
}

#[test]
fn cleared_crypto_chart_does_not_reappear_after_reopen_or_new_transactions() {
    let mut fixture = Fixture::new();
    fixture.id = save_asset(
        &mut fixture.store,
        None,
        AssetInput {
            symbol: "BTC".into(),
            name: "Bitcoin".into(),
            kind: AssetKind::Crypto,
            yahoo_symbol: Some("BTC-USD".into()),
            tradingview_symbol: Some("CRYPTO:BTCUSD".into()),
            valuation_currency: None,
        },
    )
    .unwrap();
    let mut input = fixture.input();
    input.yahoo_symbol = None;
    input.tradingview_symbol = None;
    save_asset(&mut fixture.store, Some(fixture.id), input).unwrap();
    fixture.store.save().unwrap();
    let mut fixture = fixture.reopen();
    assert!(fixture.asset().tradingview_symbol.is_none());
    let mut transaction = stock_transaction(&fixture.store);
    transaction.base_asset_id = fixture.id;
    add_manual_transaction(&mut fixture.store, transaction).unwrap();
    assert!(fixture.asset().tradingview_symbol.is_none());
    assert!(fixture.asset().yahoo_symbol.is_none());
}

#[test]
fn existing_legacy_duplicate_symbols_can_still_have_their_metadata_edited() {
    let mut fixture = Fixture::new();
    fixture.id = support::duplicate_asset(
        &mut fixture.store,
        fixture.id,
        "Legacy custom asset",
        AssetKind::Custom,
    );
    let mut input = fixture.input();
    input.name = "Edited custom asset".into();
    save_asset(&mut fixture.store, Some(fixture.id), input).unwrap();
    assert_eq!(fixture.asset().name, "Edited custom asset");
}

#[test]
fn rejects_symbol_changes_and_duplicate_creation_without_partial_changes() {
    let mut fixture = Fixture::new();
    let original = serde_json::to_value(&fixture.store.data).unwrap();
    let mut input = fixture.input();
    input.symbol = "EUR".into();
    assert!(save_asset(&mut fixture.store, Some(fixture.id), input).is_err());
    assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), original);
    let mut input = fixture.input();
    input.symbol = "APPLE".into();
    let error = save_asset(&mut fixture.store, Some(fixture.id), input).unwrap_err();
    assert!(error.to_string().contains("symbols cannot be changed"));
    let another = fixture.input();
    assert!(save_asset(&mut fixture.store, None, another).is_err());
}

#[test]
fn legacy_symbol_aliases_are_ignored_and_removed_when_the_store_is_saved() {
    let fixture = Fixture::new();
    let mut old_data = serde_json::to_value(&fixture.store.data).unwrap();
    old_data["assets"][0]["previous_symbols"] = serde_json::json!(["OLD"]);
    std::fs::write(fixture.store.path(), serde_json::to_vec(&old_data).unwrap()).unwrap();
    let fixture = fixture.reopen();
    assert!(fixture.store.asset_by_symbol("OLD").is_none());
    fixture.store.save().unwrap();
    let saved: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(fixture.store.path()).unwrap()).unwrap();
    assert!(saved["assets"][0].get("previous_symbols").is_none());
}

#[test]
fn editing_metadata_preserves_the_case_of_existing_symbols() {
    let mut fixture = Fixture::new();
    fixture.store.data.assets[0].symbol = "aapl".into();
    let mut input = fixture.input();
    input.name = "Updated display name".into();
    save_asset(&mut fixture.store, Some(fixture.id), input).unwrap();
    assert_eq!(fixture.asset().symbol, "aapl");
    assert_eq!(fixture.asset().name, "Updated display name");
}

#[test]
fn currency_codes_and_currency_roles_cannot_be_broken_by_editing() {
    let mut fixture = Fixture::new();
    let eur = fixture.store.asset_by_symbol("EUR").unwrap().clone();
    let mut input = AssetInput {
        symbol: "EURO".into(),
        name: eur.name.clone(),
        kind: AssetKind::Fiat,
        yahoo_symbol: None,
        tradingview_symbol: None,
        valuation_currency: None,
    };
    let error = save_asset(&mut fixture.store, Some(eur.id), input.clone()).unwrap_err();
    assert!(error.to_string().contains("symbols cannot be changed"));
    input.symbol = "EUR".into();
    input.kind = AssetKind::Stock;
    assert!(save_asset(&mut fixture.store, Some(eur.id), input).is_err());
    assert_eq!(
        fixture.store.asset_by_symbol("EUR").unwrap().kind,
        AssetKind::Fiat
    );
}

#[test]
fn intrinsic_valuation_is_explicit_and_can_be_cleared() {
    let mut fixture = Fixture::new();
    let mut input = fixture.input();
    input.kind = AssetKind::Custom;
    input.valuation_currency = Some(" eur ".into());
    save_asset(&mut fixture.store, Some(fixture.id), input).unwrap();
    let report = build_report(&fixture.store.data);
    assert_eq!(report.holdings[0].value, Some(dec!(2)));
    let mut input = fixture.input();
    input.valuation_currency = None;
    input.kind = AssetKind::Stock;
    save_asset(&mut fixture.store, Some(fixture.id), input).unwrap();
    assert_eq!(
        build_report(&fixture.store.data).holdings[0].value,
        Some(dec!(200))
    );
}

#[test]
fn validates_provider_identifiers_and_crypto_quote_currency() {
    let mut fixture = Fixture::new();
    for symbol in ["AAPL bad", "AAPL?query=bad"] {
        let mut input = fixture.input();
        input.yahoo_symbol = Some(symbol.into());
        assert!(save_asset(&mut fixture.store, Some(fixture.id), input).is_err());
    }
    let mut input = fixture.input();
    input.tradingview_symbol = Some("AAPL".into());
    assert!(save_asset(&mut fixture.store, Some(fixture.id), input).is_err());
    let mut input = fixture.input();
    input.kind = AssetKind::Crypto;
    input.yahoo_symbol = Some("AAPL-EUR".into());
    assert!(save_asset(&mut fixture.store, Some(fixture.id), input).is_err());
}

#[test]
fn creates_assets_without_transactions_and_defaults_the_name_to_the_symbol() {
    let mut fixture = Fixture::new();
    let before = fixture.store.data.transactions.len();
    let input = AssetInput {
        symbol: " msft ".into(),
        name: String::new(),
        kind: AssetKind::Stock,
        yahoo_symbol: Some(" MSFT ".into()),
        tradingview_symbol: Some(" NASDAQ:MSFT ".into()),
        valuation_currency: None,
    };
    let id = save_asset(&mut fixture.store, None, input).unwrap();
    let asset = fixture
        .store
        .data
        .assets
        .iter()
        .find(|asset| asset.id == id)
        .unwrap();
    assert_eq!(asset.name, "MSFT");
    assert_eq!(asset.yahoo_symbol.as_deref(), Some("MSFT"));
    assert_eq!(fixture.store.data.transactions.len(), before);
}

#[test]
fn quote_and_fee_selection_respects_preconfigured_currency_types() {
    let mut fixture = Fixture::new();
    let input = AssetInput {
        symbol: "JPY".into(),
        name: "Japanese yen".into(),
        kind: AssetKind::Fiat,
        yahoo_symbol: None,
        tradingview_symbol: None,
        valuation_currency: None,
    };
    let jpy = save_asset(&mut fixture.store, None, input).unwrap();
    let mut transaction = stock_transaction(&fixture.store);
    transaction.quote_asset_id = Some(jpy);
    transaction.fee_asset_id = Some(jpy);
    transaction.fee_amount = Some(dec!(10));
    add_manual_transaction(&mut fixture.store, transaction).unwrap();
    let transaction = fixture.store.data.transactions.last().unwrap();
    assert_eq!(transaction.quote_asset_id, Some(jpy));
    assert_eq!(transaction.fee_asset_id, Some(jpy));
    assert_eq!(fixture.store.data.assets.len(), 3);
    assert_eq!(
        fixture.store.asset_by_symbol("JPY").unwrap().kind,
        AssetKind::Fiat
    );
}

fn fetched_batch(id: Id) -> PriceBatch {
    PriceBatch {
        prices: vec![Price {
            asset_id: id,
            timestamp: Utc::now(),
            price: dec!(999),
            currency: "EUR".into(),
            source: "yahoo".into(),
        }],
        summary: SyncSummary {
            updated: 1,
            unsupported: 0,
        },
    }
}

#[test]
fn refresh_discards_quotes_when_provider_settings_changed_during_the_fetch() {
    let mut fixture = Fixture::new();
    let snapshot = fixture.store.data.clone();
    let mut input = fixture.input();
    input.yahoo_symbol = Some("OTHER".into());
    save_asset(&mut fixture.store, Some(fixture.id), input).unwrap();
    let summary = merge_price_batch(
        &mut fixture.store.data,
        &snapshot,
        fetched_batch(fixture.id),
    );
    assert_eq!(summary.updated, 0);
    assert_eq!(fixture.store.data.prices.len(), 1);
    assert_eq!(fixture.store.data.prices[0].price, dec!(100));
}

#[test]
fn refresh_keeps_quotes_and_other_edits_when_only_display_metadata_changed() {
    let mut fixture = Fixture::new();
    let snapshot = fixture.store.data.clone();
    let mut input = fixture.input();
    input.name = "Edited during refresh".into();
    save_asset(&mut fixture.store, Some(fixture.id), input).unwrap();
    let summary = merge_price_batch(
        &mut fixture.store.data,
        &snapshot,
        fetched_batch(fixture.id),
    );
    assert_eq!(summary.updated, 1);
    assert_eq!(fixture.asset().name, "Edited during refresh");
    assert_eq!(fixture.store.data.prices.len(), 2);
}

#[test]
fn usd_cannot_be_created_with_a_non_cash_type() {
    let mut fixture = Fixture::new();
    let before = serde_json::to_value(&fixture.store.data).unwrap();
    for kind in [AssetKind::Crypto, AssetKind::Stock, AssetKind::Custom] {
        let input = AssetInput {
            symbol: " usd ".into(),
            name: "US dollar".into(),
            kind,
            yahoo_symbol: None,
            tradingview_symbol: None,
            valuation_currency: None,
        };
        let error = save_asset(&mut fixture.store, None, input).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("USD must use the Cash asset type")
        );
        assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), before);
    }
}

#[test]
fn usd_type_edits_are_rejected_atomically_and_existing_bad_types_can_be_repaired() {
    let mut fixture = Fixture::new();
    let usd = support::create_asset(&mut fixture.store, "USD", "US dollar", AssetKind::Fiat);
    let input = AssetInput {
        symbol: "USD".into(),
        name: "US dollar".into(),
        kind: AssetKind::Crypto,
        yahoo_symbol: None,
        tradingview_symbol: None,
        valuation_currency: None,
    };
    let before = serde_json::to_value(&fixture.store.data).unwrap();
    let error = save_asset(&mut fixture.store, Some(usd), input.clone()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("USD must use the Cash asset type")
    );
    assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), before);

    // Simulate an existing store written before this validation rule.
    fixture
        .store
        .data
        .assets
        .iter_mut()
        .find(|asset| asset.id == usd)
        .unwrap()
        .kind = AssetKind::Crypto;
    let mut repaired = input;
    repaired.kind = AssetKind::Fiat;
    save_asset(&mut fixture.store, Some(usd), repaired).unwrap();
    assert_eq!(
        fixture.store.asset_by_symbol("USD").unwrap().kind,
        AssetKind::Fiat
    );
}

pub mod support;
