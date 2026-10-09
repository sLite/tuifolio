use axum::http::StatusCode;
use rust_decimal_macros::dec;

use super::{
    fixture::{TestApp, edit_revision, html},
    transaction_editing::form_body,
};
use crate::web::forms::TransactionForm;

const ASSET_BODY: &str =
    "symbol=BTC&name=Bitcoin&kind=Crypto&yahoo_symbol=BTC-USD&tradingview_symbol=CRYPTO%3ABTCUSD";

async fn transaction_form(fixture: &TestApp) -> (String, TransactionForm) {
    let transaction = &fixture.persisted().transactions[0];
    let path = format!("/transactions/{}/edit", transaction.id);
    let form = TransactionForm::from_transaction(transaction);
    let page = html(fixture.get(&path).await, StatusCode::OK).await;
    assert_eq!(edit_revision(&page), form.expected_revision);
    (path, form)
}

async fn assert_unchanged(fixture: &TestApp, before: &serde_json::Value, bytes: &[u8]) {
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), *before);
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        *before
    );
    assert_eq!(std::fs::read(&fixture.path).unwrap(), bytes);
}

#[tokio::test]
async fn stale_transaction_edits_preserve_drafts_and_never_overwrite_saved_quantities() {
    for htmx in [None, Some("true")] {
        let fixture = TestApp::new(true);
        let (path, mut first) = transaction_form(&fixture).await;
        let mut stale = first.clone();
        first.base_amount = "20".into();
        assert_eq!(
            fixture.post(&path, &form_body(&first)).await.status(),
            StatusCode::SEE_OTHER
        );
        stale.notes = "Keep my unsaved notes <script>draft</script>".into();
        let before = serde_json::to_value(fixture.persisted()).unwrap();
        let bytes = std::fs::read(&fixture.path).unwrap();
        let body = form_body(&stale);

        // Retrying the same stale form must not acquire the latest version.
        for _ in 0..2 {
            let page = html(
                fixture.request("POST", &path, &body, htmx).await,
                StatusCode::CONFLICT,
            )
            .await;
            assert!(page.contains("changed after you opened the form"));
            assert!(page.contains("Copy your edits"));
            assert!(page.contains("Keep my unsaved notes"));
            assert!(!page.contains("<script>draft</script>"));
            assert!(page.contains("name=\"base_amount\" value=\"0.1\""));
            assert!(page.contains("id=\"app\""));
            assert_eq!(edit_revision(&page), stale.expected_revision);
            assert_unchanged(&fixture, &before, &bytes).await;
        }
        assert_eq!(fixture.persisted().transactions[0].base_amount, dec!(20));

        // A reload lets the user reapply their notes to the corrected quantity.
        let (_, mut current) = transaction_form(&fixture).await;
        current.notes = stale.notes;
        assert_eq!(
            fixture.post(&path, &form_body(&current)).await.status(),
            StatusCode::SEE_OTHER
        );
        assert_eq!(fixture.persisted().transactions[0].base_amount, dec!(20));
        assert_eq!(
            fixture.persisted().transactions[0].notes.as_deref(),
            Some("Keep my unsaved notes <script>draft</script>")
        );
    }
}

#[tokio::test]
async fn stale_asset_edits_preserve_drafts_and_never_restore_old_provider_settings() {
    for htmx in [None, Some("true")] {
        let fixture = TestApp::new(true);
        let id = fixture.asset_id("BTC").await;
        let path = format!("/assets/{id}");
        let stale = fixture
            .edit_body(
                &path,
                &ASSET_BODY.replace("name=Bitcoin", "name=Unsaved%20name"),
            )
            .await;
        let expected = stale.split("expected_revision=").nth(1).unwrap();
        let changed = ASSET_BODY
            .replace("yahoo_symbol=BTC-USD", "yahoo_symbol=ALT-USD")
            .replace(
                "tradingview_symbol=CRYPTO%3ABTCUSD",
                "tradingview_symbol=CRYPTO%3AALTUSD",
            );
        assert_eq!(
            fixture.submit_edit(&path, &changed).await.status(),
            StatusCode::SEE_OTHER
        );
        let before = serde_json::to_value(fixture.persisted()).unwrap();
        let bytes = std::fs::read(&fixture.path).unwrap();
        for _ in 0..2 {
            let page = html(
                fixture.request("POST", &path, &stale, htmx).await,
                StatusCode::CONFLICT,
            )
            .await;
            assert!(page.contains("This asset changed"));
            assert!(page.contains("name=\"name\" value=\"Unsaved name\""));
            assert!(page.contains("name=\"yahoo_symbol\" value=\"BTC-USD\""));
            assert_eq!(edit_revision(&page), expected);
            assert_unchanged(&fixture, &before, &bytes).await;
        }
        let asset = fixture
            .persisted()
            .assets
            .into_iter()
            .find(|a| a.id == id)
            .unwrap();
        assert_eq!(asset.yahoo_symbol.as_deref(), Some("ALT-USD"));
        assert_eq!(asset.tradingview_symbol.as_deref(), Some("CRYPTO:ALTUSD"));
    }
}

#[tokio::test]
async fn simultaneous_transaction_edits_allow_only_one_changed_save() {
    let fixture = TestApp::new(true);
    let (path, mut first) = transaction_form(&fixture).await;
    let mut second = first.clone();
    first.base_amount = "20".into();
    second.base_amount = "30".into();
    let first = form_body(&first);
    let second = form_body(&second);
    let (a, b) = tokio::join!(fixture.post(&path, &first), fixture.post(&path, &second));
    let mut statuses = [a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [303, 409]);
    let data = fixture.persisted();
    assert!([dec!(20), dec!(30)].contains(&data.transactions[0].base_amount));
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        serde_json::to_value(data).unwrap()
    );
}

#[tokio::test]
async fn simultaneous_asset_edits_allow_only_one_changed_save() {
    let fixture = TestApp::new(true);
    let id = fixture.asset_id("BTC").await;
    let path = format!("/assets/{id}");
    let first = fixture.edit_body(&path, ASSET_BODY).await;
    let second = first.replace("name=Bitcoin", "name=Second");
    let first = first.replace("name=Bitcoin", "name=First");
    let (a, b) = tokio::join!(fixture.post(&path, &first), fixture.post(&path, &second));
    let mut statuses = [a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [303, 409]);
    let data = fixture.persisted();
    assert!(
        ["First", "Second"].contains(
            &data
                .assets
                .iter()
                .find(|a| a.id == id)
                .unwrap()
                .name
                .as_str()
        )
    );
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        serde_json::to_value(data).unwrap()
    );
}

#[tokio::test]
async fn missing_invalid_or_wrong_record_versions_cannot_bypass_conflict_checks() {
    let fixture = TestApp::new(true);
    let (path, form) = transaction_form(&fixture).await;
    let other = &fixture.persisted().transactions[1];
    let other_revision = TransactionForm::from_transaction(other).expected_revision;
    let before = serde_json::to_value(fixture.persisted()).unwrap();
    let bytes = std::fs::read(&fixture.path).unwrap();
    for expected in ["", "invalid", &other_revision] {
        let mut submitted = form.clone();
        submitted.expected_revision = expected.into();
        submitted.notes = "Unsaved transaction notes".into();
        let page = html(
            fixture.post(&path, &form_body(&submitted)).await,
            StatusCode::CONFLICT,
        )
        .await;
        assert!(page.contains("Unsaved transaction notes"));
        assert_eq!(edit_revision(&page), expected);
        assert_unchanged(&fixture, &before, &bytes).await;
    }

    let id = fixture.asset_id("BTC").await;
    let path = format!("/assets/{id}");
    for suffix in ["", "&expected_revision=invalid"] {
        let page = html(
            fixture.post(&path, &(ASSET_BODY.to_owned() + suffix)).await,
            StatusCode::CONFLICT,
        )
        .await;
        assert!(page.contains("This asset changed"));
        assert_unchanged(&fixture, &before, &bytes).await;
    }
}

#[tokio::test]
async fn unrelated_edits_and_price_updates_do_not_invalidate_record_versions() {
    let fixture = TestApp::new(true);
    let (transaction_path, mut form) = transaction_form(&fixture).await;
    let id = fixture.asset_id("BTC").await;
    let asset_path = format!("/assets/{id}");
    let asset_body = fixture
        .edit_body(
            &asset_path,
            &ASSET_BODY.replace("name=Bitcoin", "name=Updated"),
        )
        .await;
    fixture
        .state
        .edit(|store| {
            store.data.config.selected_base_currency = "USD".into();
            store.data.transactions[1].notes = Some("Unrelated deposit edit".into());
            crate::price_sync::add_manual_price(store, "USD", dec!(0.92), "EUR")
        })
        .await
        .unwrap();
    form.notes = "Valid notes".into();
    assert_eq!(
        fixture
            .post(&transaction_path, &form_body(&form))
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        fixture.post(&asset_path, &asset_body).await.status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn revisions_detect_timestamp_changes_below_the_displayed_precision() {
    let fixture = TestApp::new(true);
    fixture
        .state
        .edit(|store| {
            store.data.transactions[0].timestamp = "2026-10-08T12:30:14.123456789Z".parse()?;
            Ok(())
        })
        .await
        .unwrap();
    let (path, form) = transaction_form(&fixture).await;
    fixture
        .state
        .edit(|store| {
            store.data.transactions[0].timestamp = "2026-10-08T12:30:14.123456788Z".parse()?;
            Ok(())
        })
        .await
        .unwrap();
    let page = html(
        fixture.post(&path, &form_body(&form)).await,
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(edit_revision(&page), form.expected_revision);
    assert_eq!(
        fixture.persisted().transactions[0].timestamp.to_rfc3339(),
        "2026-10-08T12:30:14.123456788+00:00"
    );
}
