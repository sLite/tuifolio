use axum::http::StatusCode;
use rust_decimal_macros::dec;

use super::fixture::{TestApp, duplicate_asset, html};

#[tokio::test]
async fn manual_price_form_lives_only_on_saved_assets_without_yahoo_symbols() {
    let fixture = TestApp::new(true);
    let usd = fixture.asset_id("USD").await;
    let btc = fixture.asset_id("BTC").await;
    for path in ["/assets", "/assets/new"] {
        let page = html(fixture.get(path).await, StatusCode::OK).await;
        assert!(!page.contains("id=\"manual-price\""));
        assert!(!page.contains("#manual-price"));
    }
    let page = html(fixture.get(&format!("/assets/{usd}")).await, StatusCode::OK).await;
    assert!(page.contains(&format!("action=\"/assets/{usd}/prices\"")));
    assert!(!page.contains("name=\"asset_id\""));
    assert!(!page.contains("Set a manual price"));
    let page = html(fixture.get(&format!("/assets/{btc}")).await, StatusCode::OK).await;
    assert!(!page.contains("id=\"manual-price\""));
    assert!(page.contains("clear the Yahoo symbol and save the asset first"));
}

#[tokio::test]
async fn invalid_prices_preserve_input_in_the_asset_editor() {
    let fixture = TestApp::new(true);
    let usd = fixture.asset_id("USD").await;
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    for value in ["-3", "invalid"] {
        let page = html(
            fixture
                .post(
                    &format!("/assets/{usd}/prices"),
                    &format!("price={value}&currency=gbp"),
                )
                .await,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
        assert!(page.contains(&format!("name=\"price\" value=\"{value}\"")));
        assert!(page.contains("name=\"currency\" value=\"gbp\""));
        assert!(page.contains("USD · Asset"));
        assert!(page.contains("role=\"alert\""));
        assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
    }
}

#[tokio::test]
async fn prices_use_the_route_asset_and_redirect_back_to_its_editor() {
    let fixture = TestApp::new(true);
    let usd = fixture.asset_id("USD").await;
    let btc = fixture.asset_id("BTC").await;
    let response = fixture
        .post(
            &format!("/assets/{usd}/prices"),
            &format!("asset_id={btc}&price=90000.123456789&currency=eur"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()["location"],
        format!("/assets/{usd}?notice=price-added")
    );
    let data = fixture.persisted();
    let price = data.prices.last().unwrap();
    assert_eq!(price.price, dec!(90000.123456789));
    assert_eq!(price.currency, "EUR");
    assert_eq!(price.asset_id, usd);
    let page = html(
        fixture
            .get(&format!("/assets/{usd}?notice=price-added"))
            .await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("Manual price saved."));
    assert!(page.contains("90000.123456789") || page.contains("90,000.12"));
}

#[tokio::test]
async fn manual_prices_distinguish_assets_with_the_same_symbol() {
    let fixture = TestApp::new(true);
    let id = fixture
        .state
        .edit(|store| Ok(duplicate_asset(store, "Custom BTC")))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .post(&format!("/assets/{id}/prices"), "price=25&currency=EUR")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(fixture.persisted().prices.last().unwrap().asset_id, id);
}

#[tokio::test]
async fn configured_yahoo_symbols_reject_forged_manual_price_requests() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    let page = html(
        fixture
            .post(&format!("/assets/{btc}/prices"), "price=25&currency=EUR")
            .await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(
        page.contains("clear the Yahoo symbol and save the asset before recording manual prices")
    );
    assert!(!page.contains("id=\"manual-price\""));
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
}

#[tokio::test]
async fn clearing_a_yahoo_symbol_enables_manual_price_recording() {
    let fixture = TestApp::new(true);
    let btc = fixture.asset_id("BTC").await;
    fixture
        .post(
            &format!("/assets/{btc}"),
            "symbol=BTC&name=Bitcoin&kind=Crypto&yahoo_symbol=",
        )
        .await;
    let page = html(fixture.get(&format!("/assets/{btc}")).await, StatusCode::OK).await;
    assert!(page.contains("id=\"manual-price\""));
    assert_eq!(
        fixture
            .post(&format!("/assets/{btc}/prices"), "price=25&currency=EUR")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn htmx_price_saves_navigate_to_the_asset_editor() {
    let fixture = TestApp::new(true);
    let usd = fixture.asset_id("USD").await;
    let response = fixture
        .request(
            "POST",
            &format!("/assets/{usd}/prices"),
            "price=25&currency=EUR",
            Some("true"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let location: serde_json::Value =
        serde_json::from_str(response.headers()["hx-location"].to_str().unwrap()).unwrap();
    assert_eq!(
        location["path"],
        format!("/assets/{usd}?notice=price-added")
    );
}

#[tokio::test]
async fn unknown_assets_cannot_record_prices() {
    let fixture = TestApp::new(true);
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    assert_eq!(
        fixture
            .post("/assets/99999/prices", "price=25&currency=EUR")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
}

#[tokio::test]
async fn failed_price_saves_roll_back_and_preserve_input() {
    let fixture = TestApp::new(true);
    let usd = fixture.asset_id("USD").await;
    let original = serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap();
    std::fs::rename(
        fixture.path.parent().unwrap(),
        fixture.directory.path().join("archive"),
    )
    .unwrap();
    std::fs::File::create(fixture.path.parent().unwrap()).unwrap();
    let page = html(
        fixture
            .post(
                &format!("/assets/{usd}/prices"),
                "price=123.45&currency=gbp",
            )
            .await,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert!(page.contains("Your edit was not saved"));
    assert!(page.contains("name=\"price\" value=\"123.45\""));
    assert!(page.contains("name=\"currency\" value=\"gbp\""));
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        original
    );
}
