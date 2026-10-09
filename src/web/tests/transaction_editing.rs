use super::{
    super::forms::TransactionForm,
    fixture::{TestApp, html},
};
use crate::model::LedgerEffect;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use rust_decimal_macros::dec;
use tower::ServiceExt;

fn record_id(fixture: &TestApp) -> crate::model::Id {
    fixture.persisted().transactions[0].id
}

pub(super) fn form_body(form: &TransactionForm) -> String {
    [
        ("expected_revision", form.expected_revision.clone()),
        ("portfolio_id", form.portfolio_id.clone()),
        ("timestamp", form.timestamp.clone()),
        ("kind", form.kind.clone()),
        ("base_asset_id", form.base_asset_id.clone()),
        ("base_amount", form.base_amount.clone()),
        ("quote_asset_id", form.quote_asset_id.clone()),
        ("quote_amount", form.quote_amount.clone()),
        ("quote_ledger_effect", form.quote_ledger_effect.clone()),
        ("fee_asset_id", form.fee_asset_id.clone()),
        ("fee_amount", form.fee_amount.clone()),
        ("exchange", form.exchange.clone()),
        ("broker", form.broker.clone()),
        ("notes", form.notes.clone()),
    ]
    .into_iter()
    .map(|(name, value)| format!("{name}={}", urlencoding::encode(&value)))
    .collect::<Vec<_>>()
    .join("&")
}

fn existing_form(fixture: &TestApp) -> TransactionForm {
    TransactionForm::from_transaction(&fixture.persisted().transactions[0])
}

#[tokio::test]
async fn transaction_rows_link_to_editing_the_transaction_and_prefill_all_values() {
    let fixture = TestApp::new(true);
    let id = record_id(&fixture);
    let page = html(fixture.get("/transactions").await, StatusCode::OK).await;
    assert!(page.contains(&format!("href=\"/transactions/{id}/edit\"")));
    assert!(!page.contains("href=\"/assets/"));
    let page = html(
        fixture.get(&format!("/transactions/{id}/edit")).await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("Edit transaction"));
    assert!(page.contains("value=\"0.1\""));
    assert!(page.contains("&lt;script&gt;") || page.contains("&#60;script&#62;"));
    assert!(!page.contains("Asset balance effect"));
    assert!(!page.contains("base_ledger_effect"));
    assert!(page.contains(&format!("action=\"/transactions/{id}/delete\"")));
}

#[tokio::test]
async fn editing_preserves_id_and_origin_and_persists_rebuilt_balances() {
    let fixture = TestApp::new(true);
    let previous = fixture.persisted().transactions[0].clone();
    let mut form = existing_form(&fixture);
    form.base_amount = "0.333333333333333333".into();
    form.notes = "Updated transaction".into();
    let body = format!("{}&base_ledger_effect=Ignore", form_body(&form));
    let response = fixture
        .post(&format!("/transactions/{}/edit", previous.id), &body)
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response.headers()["location"]
            .to_str()
            .unwrap()
            .contains("notice=updated")
    );
    let data = fixture.persisted();
    let updated = &data.transactions[0];
    assert_eq!(updated.id, previous.id);
    assert_eq!(updated.base_amount, dec!(0.333333333333333333));
    assert_eq!(updated.source_row_hash, previous.source_row_hash);
    assert_eq!(updated.source, previous.source);
    assert_eq!(data.transactions.len(), 2);
    assert_eq!(
        asset_balance(&data, previous.base_asset_id),
        dec!(0.333333333333333333)
    );
}

fn asset_balance(data: &crate::model::StoreData, id: crate::model::Id) -> rust_decimal::Decimal {
    data.ledger_entries
        .iter()
        .filter(|entry| entry.asset_id == id)
        .map(|entry| entry.quantity_delta)
        .sum()
}

#[tokio::test]
async fn invalid_edits_preserve_input_and_do_not_modify_storage() {
    let fixture = TestApp::new(true);
    let before = serde_json::to_value(fixture.persisted()).unwrap();
    let mut form = existing_form(&fixture);
    form.base_amount = "-1".into();
    form.notes = "Keep this text".into();
    let page = html(
        fixture
            .post(
                &format!("/transactions/{}/edit", record_id(&fixture)),
                &form_body(&form),
            )
            .await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("quantity must be greater than zero"));
    assert!(page.contains("value=\"-1\""));
    assert!(page.contains("Keep this text"));
    assert!(page.contains("Save changes"));
    assert!(page.contains("transaction-sidebar"));
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), before);
}

#[tokio::test]
async fn no_op_edits_preserve_submillisecond_timestamps_and_cost_basis_only() {
    let fixture = TestApp::new(true);
    fixture
        .state
        .edit(|store| {
            store.data.transactions[0].timestamp =
                chrono::DateTime::parse_from_rfc3339("2026-10-08T12:30:14.123456789Z")?
                    .with_timezone(&chrono::Utc);
            store.data.transactions[0].quote_ledger_effect = LedgerEffect::Ignore;
            crate::ledger::rebuild_ledger(&mut store.data)
        })
        .await
        .unwrap();
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    let form = existing_form(&fixture);
    assert_eq!(
        fixture
            .post(
                &format!("/transactions/{}/edit", record_id(&fixture)),
                &form_body(&form)
            )
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
}

#[tokio::test]
async fn deletion_removes_ledger_movements_and_preserves_definitions() {
    let fixture = TestApp::new(true);
    let before = fixture.persisted();
    let id = before.transactions[0].id;
    let response = fixture
        .post(&format!("/transactions/{id}/delete"), "")
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response.headers()["location"]
            .to_str()
            .unwrap()
            .contains("notice=deleted")
    );
    let data = fixture.persisted();
    assert_eq!(data.transactions.len(), 1);
    assert!(
        !data
            .ledger_entries
            .iter()
            .any(|entry| entry.transaction_id == id)
    );
    assert_definitions_unchanged(&before, &data);
}

fn assert_definitions_unchanged(before: &crate::model::StoreData, after: &crate::model::StoreData) {
    assert_eq!(
        serde_json::to_value(&after.assets).unwrap(),
        serde_json::to_value(&before.assets).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&after.portfolios).unwrap(),
        serde_json::to_value(&before.portfolios).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&after.prices).unwrap(),
        serde_json::to_value(&before.prices).unwrap()
    );
}

#[tokio::test]
async fn edit_and_delete_failures_roll_back_memory_when_saving_fails() {
    let fixture = TestApp::new(true);
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    let id = record_id(&fixture);
    let mut form = existing_form(&fixture);
    form.notes = "Unsaved edit".into();
    break_saving(&fixture);
    let page = html(
        fixture
            .post(&format!("/transactions/{id}/edit"), &form_body(&form))
            .await,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert!(page.contains("Unsaved edit"));
    assert!(page.contains("Your edit was not saved"));
    assert_eq!(
        fixture
            .post(&format!("/transactions/{id}/delete"), "")
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        original
    );
}

fn break_saving(fixture: &TestApp) {
    std::fs::rename(
        fixture.path.parent().unwrap(),
        fixture.directory.path().join("archive"),
    )
    .unwrap();
    std::fs::File::create(fixture.path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn htmx_updates_and_deletions_navigate_to_get_pages() {
    let fixture = TestApp::new(true);
    let id = record_id(&fixture);
    let form = existing_form(&fixture);
    for (path, body, notice) in [
        (
            format!("/transactions/{id}/edit"),
            form_body(&form),
            "updated",
        ),
        (
            format!("/transactions/{id}/delete"),
            String::new(),
            "deleted",
        ),
    ] {
        let response = fixture.request("POST", &path, &body, Some("true")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let location: serde_json::Value =
            serde_json::from_str(response.headers()["hx-location"].to_str().unwrap()).unwrap();
        assert!(
            location["path"]
                .as_str()
                .unwrap()
                .contains(&format!("notice={notice}"))
        );
    }
}

#[tokio::test]
async fn missing_records_cannot_mutate_data() {
    let fixture = TestApp::new(true);
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    assert_eq!(
        fixture.get("/transactions/99999/edit").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .post(
                "/transactions/99999/edit",
                &form_body(&existing_form(&fixture))
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .post("/transactions/99999/delete", "")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
}

#[tokio::test]
async fn unsafe_delete_requests_cannot_mutate_data() {
    let fixture = TestApp::new(true);
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    let path = format!("/transactions/{}/delete", record_id(&fixture));
    assert_eq!(
        fixture.get(&path).await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("Host", "127.0.0.1:3000")
        .header("Origin", "http://example.com")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        fixture.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
}

#[tokio::test]
async fn invalid_unchanged_records_must_be_corrected_before_editing() {
    let fixture = TestApp::new(true);
    fixture
        .state
        .edit(|store| {
            store.data.transactions[0].quote_amount = Some(dec!(-1));
            Ok(())
        })
        .await
        .unwrap();
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    let id = record_id(&fixture);
    let mut form = existing_form(&fixture);
    form.notes = "Notes-only edit".into();
    let page = html(
        fixture
            .post(&format!("/transactions/{id}/edit"), &form_body(&form))
            .await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("quote amount cannot be negative"));
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
    form.quote_amount = "100".into();
    assert_eq!(
        fixture
            .post(&format!("/transactions/{id}/edit"), &form_body(&form))
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn automatic_metadata_flags_do_not_infer_provider_symbols_in_the_editor() {
    let fixture = TestApp::with_assets();
    fixture
        .state
        .edit(|store| {
            store.data.assets[0].metadata_source = crate::model::AssetMetadataSource::Automatic;
            store.data.assets[0].yahoo_symbol = None;
            store.data.assets[0].tradingview_symbol = None;
            Ok(())
        })
        .await
        .unwrap();
    let id = fixture.persisted().assets[0].id;
    let page = html(fixture.get(&format!("/assets/{id}")).await, StatusCode::OK).await;
    assert!(page.contains("name=\"yahoo_symbol\" value=\"\""));
    assert!(page.contains("name=\"tradingview_symbol\" value=\"\""));
}
