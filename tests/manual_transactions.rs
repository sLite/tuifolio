pub mod support;

use rust_decimal::Decimal;
use support::{create_asset, create_portfolio, duplicate_asset, temp_store};
use tuifolio::{
    accounting::build_report,
    assets::{AssetInput, save_asset},
    model::{AssetKind, LedgerEffect, Transaction},
    price_sync::add_manual_price,
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

fn buy_input(store: &mut Store, symbol: &str, kind: AssetKind) -> ManualTransactionInput {
    let base_asset_id = create_asset(store, symbol, symbol, kind);
    let quote_asset_id = create_asset(store, "USD", "US dollar", AssetKind::Fiat);
    let portfolio_id = create_portfolio(store, "Manual");
    ManualTransactionInput {
        portfolio_id,
        timestamp: chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        kind: tuifolio::model::TransactionKind::Buy,
        base_asset_id,
        base_amount: Decimal::ONE,
        quote_asset_id: Some(quote_asset_id),
        quote_amount: Some(Decimal::from(100)),
        quote_ledger_effect: LedgerEffect::Ignore,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
    }
}

#[test]
fn manual_buy_uses_quote_amount_for_pnl_without_cash_holding() {
    let mut store = temp_store();
    store.data.config.selected_base_currency = "USD".into();
    let mut input = buy_input(&mut store, "BTC", AssetKind::Crypto);
    input.portfolio_id = create_portfolio(&mut store, "Crypto");
    input.quote_amount = Some(Decimal::from(10000));
    let assets_before = serde_json::to_value(&store.data.assets).unwrap();
    let result = add_manual_transaction(&mut store, input).unwrap();
    add_manual_price(&mut store, "BTC", Decimal::from(50000), "USD").unwrap();
    let report = build_report(&store.data);
    assert!(
        report
            .holdings
            .iter()
            .any(|row| row.asset_id == result.asset_id
                && row.quantity == Decimal::ONE
                && row.net_invested == Some(Decimal::from(10000)))
    );
    assert!(
        !report
            .holdings
            .iter()
            .any(|row| row.symbol == "USD" && row.portfolio == "Crypto")
    );
    assert_eq!(
        serde_json::to_value(&store.data.assets).unwrap(),
        assets_before
    );
}

#[test]
fn manual_buy_can_post_cash_and_fee_movements() {
    let mut store = temp_store();
    let mut input = buy_input(&mut store, "ABC", AssetKind::Stock);
    let usd = input.quote_asset_id.unwrap();
    input.quote_ledger_effect = LedgerEffect::Post;
    input.fee_asset_id = Some(usd);
    input.fee_amount = Some(Decimal::from(2));
    input.exchange = Some("NYSE".into());
    let result = add_manual_transaction(&mut store, input).unwrap();
    let balance: Decimal = store
        .data
        .ledger_entries
        .iter()
        .filter(|entry| entry.portfolio_id == result.portfolio_id && entry.asset_id == usd)
        .map(|entry| entry.quantity_delta)
        .sum();
    assert_eq!(balance, Decimal::from(-102));
}

#[test]
fn manual_transaction_rejects_negative_fees() {
    let mut store = temp_store();
    let mut input = buy_input(&mut store, "ABC", AssetKind::Stock);
    input.fee_asset_id = input.quote_asset_id;
    input.fee_amount = Some(Decimal::from(-2));
    let error = add_manual_transaction(&mut store, input).unwrap_err();
    assert!(error.to_string().contains("fee amount"));
    assert!(store.data.transactions.is_empty());
}

#[test]
fn manual_transaction_rejects_unknown_assets_on_every_side_without_partial_changes() {
    let mut store = temp_store();
    let input = buy_input(&mut store, "BTC", AssetKind::Crypto);
    let before = serde_json::to_value(&store.data).unwrap();
    for side in ["base", "quote", "fee"] {
        let mut invalid = input.clone();
        match side {
            "base" => invalid.base_asset_id = 999,
            "quote" => invalid.quote_asset_id = Some(999),
            _ => {
                invalid.fee_asset_id = Some(999);
                invalid.fee_amount = Some(Decimal::ONE);
            }
        }
        assert!(
            add_manual_transaction(&mut store, invalid)
                .unwrap_err()
                .to_string()
                .contains("does not exist")
        );
        assert_eq!(serde_json::to_value(&store.data).unwrap(), before);
    }
}

fn broken_transaction(input: ManualTransactionInput) -> Transaction {
    Transaction {
        id: 9999,
        portfolio_id: 8888,
        timestamp: input.timestamp,
        kind: input.kind,
        base_asset_id: 999,
        base_amount: input.base_amount,
        quote_asset_id: None,
        quote_amount: None,
        quote_ledger_effect: LedgerEffect::Post,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
        source: "test".into(),
        source_row_hash: "broken".into(),
    }
}

#[test]
fn manual_transaction_does_not_partially_apply_when_ledger_rebuild_fails() {
    let mut store = temp_store();
    let input = buy_input(&mut store, "ABC", AssetKind::Stock);
    store
        .data
        .transactions
        .push(broken_transaction(input.clone()));
    let before = serde_json::to_value(&store.data).unwrap();
    let error = add_manual_transaction(&mut store, input).unwrap_err();
    assert!(error.to_string().contains("missing"));
    assert_eq!(serde_json::to_value(&store.data).unwrap(), before);
}

#[test]
fn manual_transaction_preserves_existing_asset_market_symbols() {
    let mut store = temp_store();
    let input = buy_input(&mut store, "ABC", AssetKind::Stock);
    save_asset(
        &mut store,
        Some(input.base_asset_id),
        AssetInput {
            symbol: "ABC".into(),
            name: "ABC Corp".into(),
            kind: AssetKind::Stock,
            yahoo_symbol: Some("ABC.CUSTOM".into()),
            tradingview_symbol: Some("CUSTOM:ABC".into()),
            valuation_currency: None,
        },
    )
    .unwrap();
    let before = serde_json::to_value(&store.data.assets).unwrap();
    add_manual_transaction(&mut store, input).unwrap();
    assert_eq!(serde_json::to_value(&store.data.assets).unwrap(), before);
}

#[test]
fn manual_transaction_picks_the_requested_id_when_symbols_are_duplicated() {
    let mut store = temp_store();
    let mut input = buy_input(&mut store, "BTC", AssetKind::Crypto);
    let custom = duplicate_asset(
        &mut store,
        input.base_asset_id,
        "Custom Bitcoin",
        AssetKind::Custom,
    );
    input.base_asset_id = custom;
    let result = add_manual_transaction(&mut store, input).unwrap();
    assert_eq!(result.asset_id, custom);
    assert_eq!(store.data.transactions[0].base_asset_id, custom);
    assert_eq!(store.data.assets.len(), 3);
}

#[test]
fn transactions_keep_the_selected_id_after_asset_metadata_is_updated() {
    let mut store = temp_store();
    let input = buy_input(&mut store, "ABC", AssetKind::Stock);
    let id = input.base_asset_id;
    let edited = AssetInput {
        symbol: "ABC".into(),
        name: "Updated fund".into(),
        kind: AssetKind::Fund,
        yahoo_symbol: None,
        tradingview_symbol: None,
        valuation_currency: None,
    };
    save_asset(&mut store, Some(id), edited).unwrap();
    add_manual_transaction(&mut store, input).unwrap();
    assert_eq!(store.data.transactions[0].base_asset_id, id);
    assert_eq!(store.asset_by_symbol("ABC").unwrap().kind, AssetKind::Fund);
}

#[test]
fn fee_asset_and_amount_must_be_supplied_together() {
    let mut store = temp_store();
    let input = buy_input(&mut store, "ABC", AssetKind::Stock);
    let mut missing_amount = input.clone();
    missing_amount.fee_asset_id = input.quote_asset_id;
    assert!(
        add_manual_transaction(&mut store, missing_amount)
            .unwrap_err()
            .to_string()
            .contains("fee amount is required")
    );
    let mut missing_asset = input;
    missing_asset.fee_amount = Some(Decimal::ONE);
    assert!(
        add_manual_transaction(&mut store, missing_asset)
            .unwrap_err()
            .to_string()
            .contains("fee asset is required")
    );
    assert!(store.data.transactions.is_empty());
}

#[test]
fn rejects_unknown_portfolios_without_implicit_creation() {
    let mut store = temp_store();
    let mut input = buy_input(&mut store, "ABC", AssetKind::Stock);
    input.portfolio_id = 999;
    let before = serde_json::to_value(&store.data).unwrap();
    let error = add_manual_transaction(&mut store, input).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("selected portfolio does not exist")
    );
    assert_eq!(serde_json::to_value(&store.data).unwrap(), before);
}

#[test]
fn selects_portfolios_by_id_even_when_legacy_names_are_duplicated() {
    let mut store = temp_store();
    let mut input = buy_input(&mut store, "ABC", AssetKind::Stock);
    let id = store.data.allocate_id();
    store.data.portfolios.push(tuifolio::model::Portfolio {
        id,
        name: "Manual".into(),
    });
    input.portfolio_id = id;
    let result = add_manual_transaction(&mut store, input).unwrap();
    assert_eq!(result.portfolio_id, id);
    assert!(
        store
            .data
            .ledger_entries
            .iter()
            .all(|entry| entry.portfolio_id == id)
    );
    assert_eq!(store.data.portfolios.len(), 2);
}
