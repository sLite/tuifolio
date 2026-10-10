use axum::http::StatusCode;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::fixture::{TestApp, configure_asset, html};
use crate::{
    model::{AssetKind, Id, LedgerRole, StockSplit, StoreData},
    transactions::add_manual_transaction,
};

const INVALID_SPLITS: &[(&str, &str, &str)] = &[
    ("2026-02-30", "4", "1"),
    ("", "4", "1"),
    ("2026-10-09", "NaN", "1"),
    ("2026-10-09", "4", "bad"),
    ("2026-10-09", "0", "1"),
    ("2026-10-09", "4", "0"),
    ("2026-10-09", "-4", "1"),
    ("2026-10-09", "4", "-1"),
    ("2026-10-09", "1", "1"),
    ("2026-10-09", "79228162514264337593543950335", "0.1"),
    ("2026-10-09", "0.0000000000000000000000000001", "100"),
];

fn assert_redirect(response: axum::response::Response, id: Id, notice: &str) {
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()["location"],
        format!("/assets/{id}?notice={notice}")
    );
}

async fn add_split(fixture: &TestApp, id: Id) -> String {
    let response = fixture
        .post(
            &format!("/assets/{id}/splits"),
            &split_body("2026-10-09", "4", "1"),
        )
        .await;
    assert_redirect(response, id, "split-saved");
    original_body(&fixture.persisted().config.stock_splits[0])
}

fn split_body(date: &str, numerator: &str, denominator: &str) -> String {
    format!("effective_date={date}&numerator={numerator}&denominator={denominator}")
}

fn original_body(split: &StockSplit) -> String {
    format!(
        "original_date={}&original_numerator={}&original_denominator={}",
        split.effective_date, split.numerator, split.denominator
    )
}

fn quantity(data: &StoreData, asset_id: Id) -> Decimal {
    data.ledger_entries
        .iter()
        .filter(|entry| entry.asset_id == asset_id)
        .map(|entry| entry.quantity_delta)
        .sum()
}

async fn stock_fixture() -> (TestApp, Id) {
    let fixture = TestApp::new(true);
    let id = fixture
        .state
        .edit(|store| Ok(configure_asset(store, "AAPL", "Apple", AssetKind::Stock)))
        .await
        .unwrap();
    let mut buy = fixture.buy_form().await;
    buy.base_asset_id = id.to_string();
    buy.base_amount = "10".into();
    fixture
        .state
        .edit(move |store| add_manual_transaction(store, buy.input()?))
        .await
        .unwrap();
    (fixture, id)
}

#[tokio::test]
async fn split_crud_rebuilds_balances_and_preserves_original_records() {
    let (fixture, id) = stock_fixture().await;
    let before = fixture.persisted();
    let path = format!("/assets/{id}/splits");
    let original = add_split(&fixture, id).await;
    assert_eq!(quantity(&fixture.persisted(), id), dec!(40));
    assert_preserved(&before, &fixture.persisted(), id);
    let edit = split_body("2026-10-09", "1", "2") + "&" + &original;
    assert_eq!(
        fixture.post(&path, &edit).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(quantity(&fixture.persisted(), id), dec!(5));
    assert_preserved(&before, &fixture.persisted(), id);
    let original = original_body(&fixture.persisted().config.stock_splits[0]);
    let response = fixture.post(&format!("{path}/delete"), &original).await;
    assert_redirect(response, id, "split-deleted");
    assert!(fixture.persisted().config.stock_splits.is_empty());
    assert_eq!(quantity(&fixture.persisted(), id), dec!(10));
    assert_preserved(&before, &fixture.persisted(), id);
}

fn assert_preserved(before: &StoreData, after: &StoreData, id: Id) {
    assert_eq!(
        serde_json::to_value(&before.transactions).unwrap(),
        serde_json::to_value(&after.transactions).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&before.prices).unwrap(),
        serde_json::to_value(&after.prices).unwrap()
    );
    let unrelated = |data: &StoreData| {
        data.ledger_entries
            .iter()
            .filter(|entry| entry.asset_id != id || !matches!(entry.role, LedgerRole::Base))
            .map(|entry| serde_json::to_value(entry).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(unrelated(before), unrelated(after));
}

#[tokio::test]
async fn split_dates_compound_and_only_adjust_transactions_before_the_utc_cutoff() {
    let (fixture, id) = stock_fixture().await;
    let path = format!("/assets/{id}/splits");
    for date in ["2026-10-08", "2026-10-09", "2026-10-10"] {
        let response = fixture.post(&path, &split_body(date, "2", "1")).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }
    assert_eq!(quantity(&fixture.persisted(), id), dec!(40));
    let original = original_body(&fixture.persisted().config.stock_splits[1]);
    let edit = split_body("2026-10-07", "2", "1") + "&" + &original;
    assert_eq!(
        fixture.post(&path, &edit).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(quantity(&fixture.persisted(), id), dec!(20));
}

async fn seed_existing_splits(fixture: &TestApp, id: Id) -> Id {
    fixture
        .state
        .edit(move |store| {
            for date in ["2026-10-09", "2026-10-10"] {
                store.data.config.stock_splits.push(StockSplit {
                    asset_id: id,
                    effective_date: date.into(),
                    numerator: dec!(4),
                    denominator: dec!(1),
                });
            }
            let mut duplicate = store
                .data
                .assets
                .iter()
                .find(|asset| asset.id == id)
                .unwrap()
                .clone();
            duplicate.id = store.data.allocate_id();
            let other_id = duplicate.id;
            store.data.assets.push(duplicate);
            Ok(other_id)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn existing_splits_are_scoped_by_id_and_sorted_newest_first() {
    let (fixture, id) = stock_fixture().await;
    let other = seed_existing_splits(&fixture, id).await;
    let page = html(fixture.get(&format!("/assets/{id}")).await, StatusCode::OK).await;
    assert!(page.contains("Stock splits <span class=\"count\">2</span>"));
    assert!(
        page.find("<span>2026-10-10</span>").unwrap()
            < page.find("<span>2026-10-09</span>").unwrap()
    );
    assert!(page.contains(&format!("formaction=\"/assets/{id}/splits/delete\"")));
    let page = html(
        fixture.get(&format!("/assets/{other}")).await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("No stock splits recorded for this asset"));
    let page = html(fixture.get("/assets/new").await, StatusCode::OK).await;
    assert!(!page.contains("stock-splits-heading"));
}

#[tokio::test]
async fn invalid_splits_preserve_input_and_never_write() {
    let (fixture, id) = stock_fixture().await;
    let before = serde_json::to_value(fixture.persisted()).unwrap();
    let path = format!("/assets/{id}/splits");
    for (date, numerator, denominator) in INVALID_SPLITS {
        let page = html(
            fixture
                .post(&path, &split_body(date, numerator, denominator))
                .await,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
        assert!(page.contains(&format!("name=\"numerator\" value=\"{numerator}\"")));
        assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), before);
        assert_eq!(
            serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn duplicate_dates_and_stale_updates_or_deletions_are_rejected() {
    let (fixture, id) = stock_fixture().await;
    let path = format!("/assets/{id}/splits");
    let add = split_body("2026-10-09", "4", "1");
    fixture.post(&path, &add).await;
    assert_eq!(
        fixture.post(&path, &add).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let original = original_body(&fixture.persisted().config.stock_splits[0]);
    let edit = split_body("2026-10-10", "2", "1") + "&" + &original;
    assert_eq!(
        fixture.post(&path, &edit).await.status(),
        StatusCode::SEE_OTHER
    );
    let before = serde_json::to_value(fixture.persisted()).unwrap();
    for (url, body) in [(path.clone(), edit), (format!("{path}/delete"), original)] {
        let page = html(
            fixture.post(&url, &body).await,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
        assert!(page.contains("this split has changed or was deleted"));
        assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), before);
    }
}

#[tokio::test]
async fn failed_rebuilds_roll_back_splits_and_balances() {
    let (fixture, id) = stock_fixture().await;
    let path = format!("/assets/{id}/splits");
    let before = serde_json::to_value(fixture.persisted()).unwrap();
    let page = html(
        fixture
            .post(
                &path,
                &split_body("2026-10-09", "79228162514264337593543950335", "1"),
            )
            .await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("split-adjusted amount is outside"));
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), before);
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        before
    );
}

#[tokio::test]
async fn failed_disk_writes_roll_back_splits_and_balances() {
    let (fixture, id) = stock_fixture().await;
    let path = format!("/assets/{id}/splits");
    let before = serde_json::to_value(fixture.persisted()).unwrap();
    std::fs::rename(
        fixture.path.parent().unwrap(),
        fixture.directory.path().join("archive"),
    )
    .unwrap();
    std::fs::File::create(fixture.path.parent().unwrap()).unwrap();
    let page = html(
        fixture
            .post(&path, &split_body("2026-10-09", "4", "1"))
            .await,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert!(page.contains("Your edit was not saved"));
    assert!(page.contains("name=\"numerator\" value=\"4\""));
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        before
    );
}

#[tokio::test]
async fn htmx_split_changes_navigate_to_the_editor() {
    let (fixture, id) = stock_fixture().await;
    let path = format!("/assets/{id}/splits");
    let response = fixture
        .request(
            "POST",
            &path,
            &split_body("2026-10-09", "4", "1"),
            Some("true"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let location: serde_json::Value =
        serde_json::from_str(response.headers()["hx-location"].to_str().unwrap()).unwrap();
    assert_eq!(location["path"], format!("/assets/{id}?notice=split-saved"));
    let original = original_body(&fixture.persisted().config.stock_splits[0]);
    let response = fixture
        .request("POST", &format!("{path}/delete"), &original, Some("true"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let location: serde_json::Value =
        serde_json::from_str(response.headers()["hx-location"].to_str().unwrap()).unwrap();
    assert_eq!(
        location["path"],
        format!("/assets/{id}?notice=split-deleted")
    );
}

#[tokio::test]
async fn unknown_assets_and_incomplete_deletions_do_not_change_splits() {
    let (fixture, id) = stock_fixture().await;
    add_split(&fixture, id).await;
    let before = serde_json::to_value(fixture.persisted()).unwrap();
    let original = original_body(&fixture.persisted().config.stock_splits[0]);
    assert_eq!(
        fixture
            .post("/assets/99999/splits", &split_body("2026-10-09", "4", "1"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .post("/assets/99999/splits/delete", &original)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture.post("/assets/1/splits/delete", "").await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), before);
}
