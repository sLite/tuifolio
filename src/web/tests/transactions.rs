use super::fixture::{TestApp, html};
use crate::model::AssetKind;
use axum::http::StatusCode;

#[tokio::test]
async fn transaction_form_uses_existing_asset_choices_and_preselects_known_ids() {
    let fixture = TestApp::with_assets();
    let btc = fixture.asset_id("BTC").await;
    let eur = fixture.asset_id("EUR").await;
    let page = html(
        fixture.get(&format!("/transactions/new?asset={btc}")).await,
        StatusCode::OK,
    )
    .await;
    for field in ["base_asset_id", "quote_asset_id", "fee_asset_id"] {
        assert!(page.contains(&format!("<select name=\"{field}\"")));
    }
    assert!(page.contains(&format!(
        "<option value=\"{btc}\" selected>BTC · Bitcoin · Crypto</option>"
    )));
    assert!(page.contains(&format!(
        "<option value=\"{eur}\" selected>EUR · Euro · Cash</option>"
    )));
    for field in [
        "asset_symbol",
        "asset_name",
        "asset_kind",
        "quote_symbol",
        "fee_symbol",
    ] {
        assert!(!page.contains(&format!("name=\"{field}\"")));
    }
}

#[tokio::test]
async fn transaction_form_requires_assets_to_be_created_first() {
    let fixture = TestApp::new(false);
    let page = html(fixture.get("/transactions/new").await, StatusCode::OK).await;
    assert!(page.contains("Create an asset in the asset directory"));
    assert!(page.contains("type=\"submit\" disabled>Save transaction"));
    assert!(page.contains("href=\"/assets/new\""));
}

#[tokio::test]
async fn choosing_cash_preselects_a_deposit_without_a_quote() {
    let fixture = TestApp::with_assets();
    let eur = fixture.asset_id("EUR").await;
    let page = html(
        fixture.get(&format!("/transactions/new?asset={eur}")).await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("value=\"Deposit\" selected"));
    assert!(page.contains(&format!(
        "<option value=\"{eur}\" selected>EUR · Euro · Cash</option>"
    )));
    assert_eq!(
        page.matches(&format!("<option value=\"{eur}\" selected>"))
            .count(),
        1
    );
}

#[tokio::test]
async fn transaction_post_rejects_unknown_ids_on_all_sides_without_creating_assets() {
    let fixture = TestApp::with_assets();
    let btc = fixture.asset_id("BTC").await;
    let eur = fixture.asset_id("EUR").await;
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    let body = fixture.buy_body().await;
    for body in [
        body.replace(&format!("base_asset_id={btc}"), "base_asset_id=99999"),
        body.replace(&format!("quote_asset_id={eur}"), "quote_asset_id=99999"),
        format!("{body}&fee_asset_id=99999&fee_amount=2"),
    ] {
        let page = html(
            fixture.post("/transactions/new", &body).await,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
        assert!(page.contains("does not exist; choose an existing asset"));
        assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
    }
}

#[tokio::test]
async fn transaction_post_cannot_change_asset_metadata_or_create_symbols() {
    let fixture = TestApp::with_assets();
    let original = serde_json::to_value(fixture.persisted().assets).unwrap();
    let body = fixture.buy_body().await
        + "&asset_symbol=FAKE&asset_name=Overwritten&asset_kind=Stock&quote_symbol=NEW&fee_symbol=NEW";
    assert_eq!(
        fixture.post("/transactions/new", &body).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        serde_json::to_value(fixture.persisted().assets).unwrap(),
        original
    );
}

#[tokio::test]
async fn duplicate_symbols_have_distinct_choices_and_transactions_use_the_selected_id() {
    let fixture = TestApp::with_assets();
    let btc = fixture.asset_id("BTC").await;
    let custom = fixture
        .state
        .edit(|store| Ok(store.asset_id("BTC", "Custom Bitcoin", AssetKind::Custom)))
        .await
        .unwrap();
    let page = html(
        fixture
            .get(&format!("/transactions/new?asset={custom}"))
            .await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains(&format!("BTC · Bitcoin · Crypto · #{btc}")));
    assert!(page.contains(&format!("BTC · Custom Bitcoin · Custom · #{custom}")));
    let body = fixture.buy_body().await.replace(
        &format!("base_asset_id={btc}"),
        &format!("base_asset_id={custom}"),
    );
    assert_eq!(
        fixture.post("/transactions/new", &body).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(fixture.persisted().transactions[0].base_asset_id, custom);
    assert_eq!(fixture.persisted().assets.len(), 4);
}

#[tokio::test]
async fn invalid_amounts_preserve_selected_asset_ids() {
    let fixture = TestApp::with_assets();
    let btc = fixture.asset_id("BTC").await;
    let eur = fixture.asset_id("EUR").await;
    let body = fixture
        .buy_body()
        .await
        .replace("base_amount=0.123456789123456789", "base_amount=-1");
    let page = html(
        fixture.post("/transactions/new", &body).await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains(&format!(
        "<option value=\"{btc}\" selected>BTC · Bitcoin · Crypto</option>"
    )));
    assert!(page.contains(&format!(
        "<option value=\"{eur}\" selected>EUR · Euro · Cash</option>"
    )));
    assert!(fixture.persisted().transactions.is_empty());
}
