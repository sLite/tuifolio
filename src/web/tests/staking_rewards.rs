use super::fixture::{TestApp, html};
use crate::{
    accounting::build_report,
    model::{LedgerEffect, TransactionKind},
    price_sync::add_manual_price_for_asset_at,
};
use axum::http::StatusCode;
use rust_decimal_macros::dec;

async fn fixture() -> TestApp {
    let f = TestApp::with_assets();
    f.state
        .edit(|store| {
            let btc = store.asset_by_symbol("BTC").unwrap().id;
            let usd = store.asset_by_symbol("USD").unwrap().id;
            store.data.prices.push(crate::model::Price {
                asset_id: btc,
                timestamp: "2026-10-01T00:00:00Z".parse().unwrap(),
                price: dec!(30000),
                currency: "USD".into(),
                source: "synthetic".into(),
            });
            add_manual_price_for_asset_at(
                store,
                usd,
                dec!(0.9),
                "EUR",
                "2026-10-01T00:00:00Z".parse().unwrap(),
            )?;
            Ok(())
        })
        .await
        .unwrap();
    f
}
async fn body(f: &TestApp, asset: u64) -> String {
    format!(
        "portfolio_id={}&timestamp=2026-10-08T21%3A10&kind=StakingReward&base_asset_id={asset}&base_amount=2&notes=staking%20reward",
        f.persisted().portfolios[0].id
    )
}

#[tokio::test]
async fn saves_zero_cost_reward_without_payment_or_receipt_income() {
    let f = fixture().await;
    let btc = f.asset_id("BTC").await;
    assert_eq!(
        f.post("/transactions/new", &body(&f, btc).await)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let data = f.persisted();
    let t = &data.transactions[0];
    assert_eq!(t.kind, TransactionKind::StakingReward);
    assert_eq!(t.quote_amount, None);
    assert_eq!(t.quote_ledger_effect, LedgerEffect::Ignore);
    assert_eq!(data.ledger_entries.len(), 1);
    let r = build_report(&data);
    assert_eq!(r.holdings[0].remaining_cost_basis, Some(dec!(0)));
    assert_eq!(r.holdings[0].unrealized_pnl, Some(dec!(54000)));
    assert_eq!(r.holdings[0].net_invested, Some(dec!(0)));
    for path in ["/".into(), format!("/portfolios/{}", data.portfolios[0].id)] {
        let page = html(f.get(&path).await, StatusCode::OK).await;
        assert!(!page.contains("Staking income"));
        assert!(!page.contains("historical prices or conversions are missing"));
        assert!(page.contains("<span class=\"private-value\">54,000.00</span>"));
    }
    let page = html(f.get("/transactions").await, StatusCode::OK).await;
    assert!(page.contains("Staking reward"));
    assert!(page.contains("Zero cost basis"));
}

#[tokio::test]
async fn reward_edit_preserves_ignored_quote_asset_metadata() {
    let f = fixture().await;
    let btc = f.asset_id("BTC").await;
    let eur = f.asset_id("EUR").await;
    f.post("/transactions/new", &body(&f, btc).await).await;
    f.state
        .edit(move |store| {
            store.data.transactions[0].quote_asset_id = Some(eur);
            Ok(())
        })
        .await
        .unwrap();
    let id = f.persisted().transactions[0].id;
    let url = format!("/transactions/{id}/edit");
    let page = html(f.get(&url).await, StatusCode::OK).await;
    assert!(page.contains("value=\"StakingReward\" selected"));
    let b = body(&f, btc).await + "&quote_ledger_effect=Ignore";
    assert_eq!(
        f.submit_edit(&url, &b).await.status(),
        StatusCode::SEE_OTHER
    );
    let data = f.persisted();
    assert_eq!(data.transactions[0].quote_asset_id, Some(eur));
    assert_eq!(data.transactions[0].source_row_hash, format!("manual:{id}"));
}

#[tokio::test]
async fn reward_validation_preserves_draft_and_missing_prices_affect_value_not_basis() {
    let f = TestApp::with_assets();
    let eur = f.asset_id("EUR").await;
    let old = std::fs::read(&f.path).unwrap();
    let page = html(
        f.post("/transactions/new", &body(&f, eur).await).await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("staking rewards require a Crypto asset"));
    assert!(page.contains("staking reward"));
    assert_eq!(std::fs::read(&f.path).unwrap(), old);
    let btc = f.asset_id("BTC").await;
    assert_eq!(
        f.post("/transactions/new", &body(&f, btc).await)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let r = build_report(&f.persisted());
    assert_eq!(r.holdings[0].remaining_cost_basis, Some(dec!(0)));
    assert_eq!(r.holdings[0].value, None);
    let page = html(f.get("/").await, StatusCode::OK).await;
    assert!(!page.contains("staking positions have unavailable income"));
    assert!(page.contains("Unavailable"));
}
