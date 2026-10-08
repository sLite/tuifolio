use super::fixture::{TestApp, html};
use crate::model::{AssetKind, AssetMetadataSource};
use axum::http::StatusCode;

fn crypto_body() -> &'static str {
    "symbol=BTC&name=Bitcoin&kind=Crypto&yahoo_symbol=BTC-USD&tradingview_symbol=CRYPTO%3ABTCUSD&valuation_currency="
}

#[tokio::test]
async fn asset_directory_searches_providers_and_links_to_editors() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    let page = html(fixture.get("/assets?search=BTC-USD").await, StatusCode::OK).await;
    assert!(page.contains(&format!("data-asset-id=\"{btc}\"")));
    assert!(page.contains(&format!("href=\"/assets/{btc}\"")));
    assert!(!page.contains("data-asset-id=\"9999\""));
    assert!(page.contains("Yahoo"));
    let page = html(fixture.get(&format!("/assets/{btc}")).await, StatusCode::OK).await;
    assert!(page.contains("CRYPTO:BTCUSD"));
    assert!(page.contains("used as a currency"));
}

#[tokio::test]
async fn asset_updates_preserve_references_and_are_visible_in_transaction_choices() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    let original = fixture.persisted();
    let body = crypto_body().replace("name=Bitcoin", "name=Bitcoin%20edited");
    let response = fixture.post(&format!("/assets/{btc}"), &body).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()["location"],
        format!("/assets/{btc}?notice=saved")
    );
    let data = fixture.persisted();
    let asset = data.assets.iter().find(|asset| asset.id == btc).unwrap();
    assert_eq!(asset.name, "Bitcoin edited");
    assert_eq!(asset.metadata_source, AssetMetadataSource::User);
    assert_eq!(
        serde_json::to_value(&data.transactions).unwrap(),
        serde_json::to_value(&original.transactions).unwrap()
    );
    let page = html(fixture.get("/transactions/new").await, StatusCode::OK).await;
    assert!(page.contains("BTC · Bitcoin edited · Crypto"));
}

#[tokio::test]
async fn invalid_edits_preserve_values_and_do_not_save() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    let body = crypto_body().replace(
        "tradingview_symbol=CRYPTO%3ABTCUSD",
        "tradingview_symbol=BAD",
    ) + "&unknown=ignored";
    let page = html(
        fixture.post(&format!("/assets/{btc}"), &body).await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("exchange-qualified TradingView symbol"));
    assert!(page.contains("value=\"BAD\""));
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
    let bad_currency = crypto_body().replace("symbol=BTC", "symbol=RENAMED");
    assert_eq!(
        fixture
            .post(&format!("/assets/{btc}"), &bad_currency)
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn clears_provider_symbols_without_defaults_being_reapplied() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    let body =
        "symbol=BTC&name=Bitcoin&kind=Crypto&yahoo_symbol=&tradingview_symbol=&valuation_currency=";
    assert_eq!(
        fixture.post(&format!("/assets/{btc}"), body).await.status(),
        StatusCode::SEE_OTHER
    );
    let data = fixture.persisted();
    let asset = data.assets.iter().find(|asset| asset.id == btc).unwrap();
    assert!(asset.yahoo_symbol.is_none());
    assert!(asset.tradingview_symbol.is_none());
    assert_eq!(asset.metadata_source, AssetMetadataSource::User);
    let page = html(fixture.get(&format!("/assets/{btc}")).await, StatusCode::OK).await;
    assert!(!page.contains("TradingView chart"));
    assert!(page.contains("name=\"yahoo_symbol\" value=\"\""));
}

#[tokio::test]
async fn creates_assets_before_transactions_and_rejects_duplicates() {
    let fixture = TestApp::new(false);
    assert_eq!(fixture.get("/assets/new").await.status(), StatusCode::OK);
    let body =
        "symbol=AAPL&name=Apple&kind=Stock&yahoo_symbol=AAPL&tradingview_symbol=NASDAQ%3AAAPL";
    let response = fixture.post("/assets/new", body).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let data = fixture.persisted();
    assert_eq!(data.assets.len(), 1);
    assert_eq!(data.assets[0].kind, AssetKind::Stock);
    assert!(data.transactions.is_empty());
    assert_eq!(
        fixture.post("/assets/new", body).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(fixture.persisted().assets.len(), 1);
}

#[tokio::test]
async fn failed_asset_saves_preserve_form_input_and_roll_back() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    std::fs::rename(
        fixture.path.parent().unwrap(),
        fixture.directory.path().join("archive"),
    )
    .unwrap();
    std::fs::File::create(fixture.path.parent().unwrap()).unwrap();
    let body = crypto_body().replace("name=Bitcoin", "name=Unsaved%20name");
    let page = html(
        fixture.post(&format!("/assets/{btc}"), &body).await,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert!(page.contains("Unsaved name"));
    assert!(page.contains("Your edit was not saved"));
    let data = fixture.state.snapshot().await.unwrap();
    assert_eq!(
        data.assets
            .iter()
            .find(|asset| asset.id == btc)
            .unwrap()
            .name,
        "Bitcoin"
    );
}

#[tokio::test]
async fn asset_forms_escape_names_and_unknown_assets_return_not_found() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    let body = crypto_body().replace(
        "name=Bitcoin",
        "name=%3Cscript%3Ealert%281%29%3C%2Fscript%3E",
    );
    assert_eq!(
        fixture
            .post(&format!("/assets/{btc}"), &body)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let page = html(fixture.get(&format!("/assets/{btc}")).await, StatusCode::OK).await;
    assert!(!page.contains("<script>alert(1)</script>"));
    assert_eq!(
        fixture.get("/assets/99999").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture.post("/assets/99999", crypto_body()).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn htmx_asset_save_navigates_to_a_get_and_manual_prices_preselect_the_asset() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    let response = fixture
        .request(
            "POST",
            &format!("/assets/{btc}"),
            crypto_body(),
            Some("true"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let location: serde_json::Value =
        serde_json::from_str(response.headers()["hx-location"].to_str().unwrap()).unwrap();
    assert_eq!(location["path"], format!("/assets/{btc}?notice=saved"));
    let page = html(
        fixture.get(&format!("/assets?asset={btc}")).await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains(&format!("<option value=\"{btc}\" selected>")));
}

#[tokio::test]
async fn base_switch_returns_to_the_asset_editor() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    let body = format!("currency=USD&return_to=%2Fassets%2F{btc}");
    let response = fixture.post("/base", &body).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], format!("/assets/{btc}"));
}

#[tokio::test]
async fn every_existing_asset_symbol_is_read_only_and_forged_changes_are_rejected() {
    let fixture = TestApp::with_assets();
    let id = fixture
        .state
        .edit(|store| Ok(store.asset_id("AAPL", "Apple", AssetKind::Stock)))
        .await
        .unwrap();
    let page = html(fixture.get(&format!("/assets/{id}")).await, StatusCode::OK).await;
    assert!(page.contains("readonly aria-describedby=\"asset-symbol-help\""));
    let body = "symbol=RENAMED&name=Updated%20display%20name&kind=Stock&yahoo_symbol=AAPL&tradingview_symbol=NASDAQ%3AAAPL";
    let page = html(
        fixture.post(&format!("/assets/{id}"), body).await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("asset symbols cannot be changed"));
    assert!(page.contains("name=\"symbol\" value=\"AAPL\""));
    let data = fixture.persisted();
    let asset = data.assets.iter().find(|asset| asset.id == id).unwrap();
    assert_eq!(asset.symbol, "AAPL");
    assert_eq!(asset.name, "Apple");
}
