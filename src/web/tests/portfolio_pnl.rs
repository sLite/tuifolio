use axum::http::StatusCode;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::fixture::{TestApp, html};
use crate::{
    assets::{AssetInput, save_asset},
    model::{AssetKind, Id, LedgerEffect, TransactionKind},
    portfolios::{PortfolioInput, create_portfolio},
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

struct PnlCase {
    id: Id,
    text: &'static str,
    tone: &'static str,
}

fn property(store: &mut Store, currency: &str) -> anyhow::Result<Id> {
    save_asset(
        store,
        None,
        AssetInput {
            symbol: "HOME".into(),
            name: "Home".into(),
            kind: AssetKind::Property,
            yahoo_symbol: None,
            tradingview_symbol: None,
            valuation_currency: Some(currency.into()),
        },
    )
}

fn acquisition(
    portfolio: Id,
    asset: Id,
    quote: Id,
    quantity: Decimal,
    cost: Decimal,
) -> ManualTransactionInput {
    ManualTransactionInput {
        portfolio_id: portfolio,
        timestamp: chrono::Utc::now(),
        kind: TransactionKind::AssetIncrease,
        base_asset_id: asset,
        base_amount: quantity,
        quote_asset_id: Some(quote),
        quote_amount: Some(cost),
        quote_ledger_effect: LedgerEffect::Ignore,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
    }
}

fn seed_portfolios(store: &mut Store) -> anyhow::Result<Vec<PnlCase>> {
    let asset = property(store, "EUR")?;
    let quote = store.asset_by_symbol("EUR").unwrap().id;
    let mut cases = Vec::new();
    for (name, quantity, text, tone) in [
        ("Gain", Some(dec!(300)), "200.00", "positive"),
        ("Loss", Some(dec!(50)), "-50.00", "negative"),
        ("Empty", None, "0.00", "muted"),
    ] {
        let id = create_portfolio(store, PortfolioInput { name: name.into() })?;
        if let Some(quantity) = quantity {
            add_manual_transaction(store, acquisition(id, asset, quote, quantity, dec!(100)))?;
        }
        cases.push(PnlCase { id, text, tone });
    }
    Ok(cases)
}

async fn assert_portfolio_pnl(fixture: &TestApp, list: &str, case: &PnlCase) {
    let link = format!("href=\"/portfolios/{}\"", case.id);
    let row = list.split("<tr>").find(|row| row.contains(&link)).unwrap();
    assert!(row.contains(&format!(
        "<td class=\"number {}\">{}</td>",
        case.tone, case.text
    )));
    let page = html(
        fixture.get(&format!("/portfolios/{}", case.id)).await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("Unrealized PnL"));
    assert!(page.contains(&format!(
        "<strong class=\"{}\">{}</strong>",
        case.tone, case.text
    )));
    assert!(page.contains("where cost basis is available"));
}

#[tokio::test]
async fn portfolio_list_and_detail_show_scoped_pnl_with_positive_negative_and_zero_tones() {
    let fixture = TestApp::with_assets();
    let cases = fixture.state.edit(seed_portfolios).await.unwrap();
    let list = html(fixture.get("/portfolios").await, StatusCode::OK).await;
    assert!(list.contains("Unrealized PnL"));
    for case in cases {
        assert_portfolio_pnl(&fixture, &list, &case).await;
    }
    let overview = html(fixture.get("/").await, StatusCode::OK).await;
    assert!(overview.contains("<strong class=\"positive\">150.00</strong>"));
}

fn seed_crypto_portfolio(store: &mut Store) -> anyhow::Result<Id> {
    store.data.config.selected_base_currency = "BTC".into();
    let asset = property(store, "BTC")?;
    let quote = store.asset_by_symbol("BTC").unwrap().id;
    let id = store.data.portfolios[0].id;
    add_manual_transaction(
        store,
        acquisition(id, asset, quote, dec!(0.00000002), dec!(0.00000001)),
    )?;
    Ok(id)
}

#[tokio::test]
async fn portfolio_pnl_uses_crypto_precision_in_the_list_and_detail() {
    let fixture = TestApp::with_assets();
    let id = fixture.state.edit(seed_crypto_portfolio).await.unwrap();
    let list = html(fixture.get("/portfolios").await, StatusCode::OK).await;
    assert_portfolio_pnl(
        &fixture,
        &list,
        &PnlCase {
            id,
            text: "0.00000001",
            tone: "positive",
        },
    )
    .await;
}
