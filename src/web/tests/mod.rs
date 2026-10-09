mod asset_quotes;
mod assets;
mod edit_conflicts;
mod fixture;
mod gifts;
mod manual_prices;
mod portfolio_pnl;
mod portfolios;
mod privacy;
mod quote_visibility;
mod stock_splits;
mod transaction_details;
mod transaction_editing;
mod transactions;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use rust_decimal_macros::dec;
use tower::ServiceExt;

use super::{
    query::{AssetScope, PageQuery},
    tables::Amount,
    views::TransactionsPage,
};
use crate::{accounting::build_report, model::LedgerEffect, transactions::add_manual_transaction};
use fixture::{TestApp, html, repeat_transactions};

#[tokio::test]
async fn renders_empty_pages_and_embedded_assets() {
    let fixture = TestApp::new(false);
    for path in [
        "/",
        "/portfolios",
        "/transactions?portfolio=&asset=",
        "/transactions/new",
        "/assets",
    ] {
        let page = html(fixture.get(path).await, StatusCode::OK).await;
        assert!(page.contains("id=\"app\""));
        assert!(page.contains("/static/htmx.min.js"));
    }
    for (name, content_type) in [
        ("style.css", "text/css"),
        ("app.js", "text/javascript"),
        ("privacy.js", "text/javascript"),
        ("htmx.min.js", "text/javascript"),
        ("plex-sans-latin.woff2", "font/woff2"),
    ] {
        let response = fixture.get(&format!("/static/{name}")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(content_type)
        );
    }
}

#[tokio::test]
async fn renders_seeded_reports_and_escapes_notes() {
    let fixture = TestApp::new(true);
    let data = fixture.state.snapshot().await.unwrap();
    for path in [
        "/".into(),
        "/assets".into(),
        format!("/portfolios/{}", data.portfolios[0].id),
    ] {
        let page = html(fixture.get(&path).await, StatusCode::OK).await;
        assert!(page.contains("BTC"));
        assert!(page.contains("30,000.00") || page.contains("30000") || page.contains("3,000.00"));
    }
    let page = html(fixture.get("/transactions").await, StatusCode::OK).await;
    assert!(
        page.contains("&lt;script&gt;") || page.contains("&#60;script&#62;"),
        "{page}"
    );
    assert!(!page.contains("<script>alert"));
    assert!(page.contains("Cost basis only"));
}

#[tokio::test]
async fn scopes_filters_to_primary_quote_and_fee_assets() {
    let fixture = TestApp::new(true);
    let eur = fixture.asset_id("EUR").await;
    let usd = fixture.asset_id("USD").await;
    for (asset, scope, count) in [
        (eur, "primary", 1),
        (eur, "any", 2),
        (usd, "primary", 0),
        (usd, "any", 1),
    ] {
        let page = html(
            fixture
                .get(&format!("/transactions?asset={asset}&scope={scope}"))
                .await,
            StatusCode::OK,
        )
        .await;
        assert!(page.contains(&format!("{count} matching entries")));
    }
    let missing = fixture.get("/transactions?portfolio=99999").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let page = html(
        fixture.get("/transactions?search=ledger").await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("1 matching entries"));
}

#[tokio::test]
async fn saves_exact_decimals_and_cost_basis_without_cash_posting() {
    let fixture = TestApp::with_assets();
    let response = fixture
        .post("/transactions/new", &fixture.buy_body().await)
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response.headers()["location"]
            .to_str()
            .unwrap()
            .starts_with("/transactions?portfolio=")
    );
    let data = fixture.persisted();
    assert_eq!(data.transactions[0].base_amount, dec!(0.123456789123456789));
    assert_eq!(data.transactions[0].quote_amount, Some(dec!(123.123456789)));
    assert_eq!(
        data.transactions[0].quote_ledger_effect,
        LedgerEffect::Ignore
    );
    assert_eq!(data.ledger_entries.len(), 1);
    assert_eq!(
        data.transactions[0].timestamp.to_rfc3339(),
        "2026-10-08T12:30:00+00:00"
    );
}

#[tokio::test]
async fn posts_cash_and_fees_through_the_transaction_form() {
    let fixture = TestApp::new(true);
    let body = fixture
        .buy_body()
        .await
        .replace("quote_ledger_effect=Ignore", "quote_ledger_effect=Post")
        + &format!(
            "&fee_asset_id={}&fee_amount=0.5",
            fixture.asset_id("EUR").await
        );
    assert_eq!(
        fixture.post("/transactions/new", &body).await.status(),
        StatusCode::SEE_OTHER
    );
    let report = build_report(&fixture.persisted());
    let eur = report.holdings.iter().find(|h| h.symbol == "EUR").unwrap();
    assert_eq!(eur.quantity, dec!(4998) - dec!(123.123456789) - dec!(0.5));
}

#[tokio::test]
async fn invalid_transactions_preserve_input_and_do_not_write() {
    let fixture = TestApp::with_assets();
    let body = fixture
        .buy_body()
        .await
        .replace("base_amount=0.123456789123456789", "base_amount=-1")
        + "&notes=keep%20this%20note";
    let page = html(
        fixture.post("/transactions/new", &body).await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("quantity must be greater than zero"));
    assert!(page.contains("keep this note"));
    assert!(page.contains("value=\"-1\""));
    assert!(fixture.persisted().transactions.is_empty());
    assert!(
        fixture
            .state
            .snapshot()
            .await
            .unwrap()
            .transactions
            .is_empty()
    );
}

#[tokio::test]
async fn malformed_decimals_and_dates_are_rejected() {
    let fixture = TestApp::with_assets();
    let body = fixture.buy_body().await;
    for body in [
        body.replace("base_amount=0.123456789123456789", "base_amount=NaN"),
        body.replace("timestamp=2026-10-08T12%3A30", "timestamp=yesterday"),
        body.replace("quote_amount=123.123456789", "quote_amount="),
    ] {
        assert_eq!(
            fixture.post("/transactions/new", &body).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert!(fixture.persisted().transactions.is_empty());
    }
}

#[tokio::test]
async fn htmx_saves_navigate_to_a_get_instead_of_reposting() {
    let fixture = TestApp::with_assets();
    let response = fixture
        .request(
            "POST",
            "/transactions/new",
            &fixture.buy_body().await,
            Some("true"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let location: serde_json::Value =
        serde_json::from_str(response.headers()["hx-location"].to_str().unwrap()).unwrap();
    assert!(
        location["path"]
            .as_str()
            .unwrap()
            .starts_with("/transactions?")
    );
    assert_eq!(location["target"], "#app");
    assert_eq!(location["select"], "#app");
}

#[tokio::test]
async fn base_currency_changes_persist_and_redirect_locally() {
    let fixture = TestApp::new(true);
    let response = fixture
        .post(
            "/base",
            "currency=BTC&return_to=%2Ftransactions%3Fsearch%3DBTC",
        )
        .await;
    assert_eq!(response.headers()["location"], "/transactions?search=BTC");
    assert_eq!(fixture.persisted().config.selected_base_currency, "BTC");
    let page = html(fixture.get("/").await, StatusCode::OK).await;
    assert!(page.contains("0.10000000"));
    let response = fixture
        .post("/base", "currency=EUR&return_to=https%3A%2F%2Fexample.com")
        .await;
    assert_eq!(response.headers()["location"], "/");
    assert_eq!(
        fixture
            .post("/base", "currency=INVALID&return_to=%2F")
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(fixture.persisted().config.selected_base_currency, "EUR");
}

#[tokio::test]
async fn rejects_cross_origin_writes_and_foreign_hosts() {
    let fixture = TestApp::with_assets();
    let body = fixture.buy_body().await;
    for origin in [None, Some("http://example.com"), Some("null")] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/transactions/new")
            .header("Host", "127.0.0.1:3000")
            .header("Content-Type", "application/x-www-form-urlencoded");
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        let response = fixture
            .app
            .clone()
            .oneshot(request.body(Body::from(body.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let request = Request::builder()
        .uri("/")
        .header("Host", "example.com:3000")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        fixture.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert!(fixture.persisted().transactions.is_empty());
}

#[tokio::test]
async fn failed_disk_writes_roll_back_in_memory_edits() {
    let fixture = TestApp::new(true);
    let archived = fixture.directory.path().join("archived");
    std::fs::rename(fixture.path.parent().unwrap(), &archived).unwrap();
    std::fs::File::create(fixture.path.parent().unwrap()).unwrap();
    let response = fixture
        .post("/transactions/new", &fixture.buy_body().await)
        .await;
    let page = html(response, StatusCode::INTERNAL_SERVER_ERROR).await;
    assert!(page.contains("Your edit was not saved"));
    assert!(page.contains("value=\"0.123456789123456789\""));
    assert_eq!(
        fixture.state.snapshot().await.unwrap().transactions.len(),
        2
    );
    let saved: crate::model::StoreData =
        serde_json::from_reader(std::fs::File::open(archived.join("store.json")).unwrap()).unwrap();
    assert_eq!(saved.transactions.len(), 2);
}

#[tokio::test]
async fn concurrent_edits_do_not_lose_transactions() {
    let fixture = TestApp::with_assets();
    let mut jobs = tokio::task::JoinSet::new();
    for index in 1..=10 {
        let state = fixture.state.clone();
        let mut form = fixture.buy_form().await;
        form.base_amount = index.to_string();
        jobs.spawn(async move {
            state
                .edit(move |store| add_manual_transaction(store, form.input()?))
                .await
                .unwrap()
        });
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap();
    }
    let data = fixture.persisted();
    assert_eq!(data.transactions.len(), 10);
    assert_eq!(
        data.transactions
            .iter()
            .map(|t| t.base_amount)
            .sum::<rust_decimal::Decimal>(),
        dec!(55)
    );
    assert_eq!(
        fixture.state.snapshot().await.unwrap().transactions.len(),
        10
    );
}

#[tokio::test]
async fn refresh_without_provider_assets_completes_and_releases_the_refresh_gate() {
    let fixture = TestApp::new(false);
    for _ in 0..2 {
        let page = html(fixture.post("/assets/sync", "").await, StatusCode::OK).await;
        assert!(page.contains("Updated 0 quotes"));
        assert!(!fixture.state.is_refreshing());
    }
}

#[tokio::test]
async fn pagination_keeps_filters_and_clamps_out_of_range_pages() {
    let fixture = TestApp::new(true);
    let data = repeat_transactions(fixture.state.snapshot().await.unwrap(), 55);
    let asset = data.assets.iter().find(|a| a.symbol == "USD").unwrap().id;
    let mut query = PageQuery {
        asset: Some(asset),
        scope: AssetScope::Any,
        page: 1,
        ..PageQuery::default()
    };
    let page = TransactionsPage::new(&data, &query);
    assert_eq!(page.transactions.len(), 50);
    assert_eq!(page.pagination.total, 56);
    assert!(
        page.pagination
            .next
            .contains(&format!("asset={asset}&scope=any"))
    );
    assert!(
        page.transactions
            .windows(2)
            .all(|pair| pair[0].id > pair[1].id)
    );
    query.page = usize::MAX;
    let page = TransactionsPage::new(&data, &query);
    assert_eq!(page.pagination.page, 2);
    assert_eq!(page.transactions.len(), 6);
}

#[test]
fn crypto_valuations_preserve_small_amounts_and_signed_grouping() {
    assert_eq!(Amount::new(Some(dec!(0.00000001)), 8).text, "0.00000001");
    assert_eq!(
        Amount::new(Some(dec!(-1234567.89)), 2).text,
        "-1,234,567.89"
    );
    assert_eq!(Amount::new(None, 2).text, "Unavailable");
}
