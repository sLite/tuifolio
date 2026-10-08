use super::fixture::{TestApp, html};
use axum::http::StatusCode;

#[tokio::test]
async fn creates_portfolios_and_preselects_them_in_transaction_forms() {
    let fixture = TestApp::with_assets();
    let response = fixture
        .post("/portfolios/new", "name=%20Long-term%20investments%20")
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let data = fixture.persisted();
    let portfolio = data.portfolios.last().unwrap();
    assert_eq!(portfolio.name, "Long-term investments");
    assert_eq!(
        response.headers()["location"],
        format!("/portfolios/{}", portfolio.id)
    );
    assert!(data.transactions.is_empty());
    let page = html(
        fixture
            .get(&format!("/transactions/new?portfolio={}", portfolio.id))
            .await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("<select name=\"portfolio_id\""));
    assert!(page.contains(&format!(
        "<option value=\"{}\" selected>Long-term investments</option>",
        portfolio.id
    )));
    assert!(!page.contains("name=\"portfolio_name\""));
    let page = html(fixture.get("/portfolios").await, StatusCode::OK).await;
    assert!(page.contains("Long-term investments"));
}

#[tokio::test]
async fn portfolio_validation_preserves_input_and_does_not_write() {
    let fixture = TestApp::with_assets();
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    let page = html(
        fixture.post("/portfolios/new", "name=%20main%20").await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("portfolio with this name already exists"));
    assert!(page.contains("value=\" main \""));
    assert_eq!(
        fixture.post("/portfolios/new", "name=").await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
}

#[tokio::test]
async fn transaction_post_rejects_unknown_portfolios_without_creating_them() {
    let fixture = TestApp::with_assets();
    let data = fixture.persisted();
    let original = serde_json::to_value(&data).unwrap();
    let body = fixture.buy_body().await.replace(
        &format!("portfolio_id={}", data.portfolios[0].id),
        "portfolio_id=99999",
    );
    let page = html(
        fixture.post("/transactions/new", &body).await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("selected portfolio does not exist"));
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
}

#[tokio::test]
async fn empty_portfolio_directory_requires_creation_before_transactions() {
    let fixture = TestApp::new(false);
    let page = html(fixture.get("/transactions/new").await, StatusCode::OK).await;
    assert!(page.contains("Create a portfolio before recording a transaction"));
    assert!(page.contains("type=\"submit\" disabled>Save transaction"));
    let page = html(fixture.get("/portfolios").await, StatusCode::OK).await;
    assert!(page.contains("href=\"/portfolios/new\""));
}

#[tokio::test]
async fn htmx_portfolio_creation_navigates_to_a_get() {
    let fixture = TestApp::new(false);
    let response = fixture
        .request(
            "POST",
            "/portfolios/new",
            "name=New%20portfolio",
            Some("true"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let location: serde_json::Value =
        serde_json::from_str(response.headers()["hx-location"].to_str().unwrap()).unwrap();
    let id = fixture.persisted().portfolios[0].id;
    assert_eq!(location["path"], format!("/portfolios/{id}"));
    assert_eq!(location["target"], "#app");
}

#[tokio::test]
async fn failed_portfolio_saves_roll_back_and_preserve_the_name() {
    let fixture = TestApp::with_assets();
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    std::fs::rename(
        fixture.path.parent().unwrap(),
        fixture.directory.path().join("archive"),
    )
    .unwrap();
    std::fs::File::create(fixture.path.parent().unwrap()).unwrap();
    let page = html(
        fixture
            .post("/portfolios/new", "name=Unsaved%20portfolio")
            .await,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert!(page.contains("value=\"Unsaved portfolio\""));
    assert!(page.contains("Your edit was not saved"));
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        original
    );
}

#[tokio::test]
async fn duplicate_legacy_names_have_distinct_choices_and_submit_by_id() {
    let fixture = TestApp::with_assets();
    let original_id = fixture.persisted().portfolios[0].id;
    let id = add_legacy_duplicate(&fixture).await;
    let page = html(
        fixture
            .get(&format!("/transactions/new?portfolio={id}"))
            .await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains(&format!("Main · #{original_id}")));
    assert!(page.contains(&format!(
        "<option value=\"{id}\" selected>Main · #{id}</option>"
    )));
    let body = fixture.buy_body().await.replace(
        &format!("portfolio_id={original_id}"),
        &format!("portfolio_id={id}"),
    );
    assert_eq!(
        fixture.post("/transactions/new", &body).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(fixture.persisted().transactions[0].portfolio_id, id);
}

async fn add_legacy_duplicate(fixture: &TestApp) -> crate::model::Id {
    fixture
        .state
        .edit(|store| {
            let id = store.data.allocate_id();
            store.data.portfolios.push(crate::model::Portfolio {
                id,
                name: "Main".into(),
            });
            Ok(id)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn portfolio_names_are_escaped_on_the_detail_page() {
    let fixture = TestApp::new(false);
    assert_eq!(
        fixture
            .post(
                "/portfolios/new",
                "name=%3Cscript%3Ealert%281%29%3C%2Fscript%3E"
            )
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let id = fixture.persisted().portfolios[0].id;
    let page = html(
        fixture.get(&format!("/portfolios/{id}")).await,
        StatusCode::OK,
    )
    .await;
    assert!(!page.contains("<script>alert(1)</script>"));
}
