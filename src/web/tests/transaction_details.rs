use axum::http::StatusCode;

use super::fixture::{TestApp, html};

async fn fixture_with_details() -> TestApp {
    let fixture = TestApp::new(true);
    fixture
        .state
        .edit(|store| {
            let transaction = &mut store.data.transactions[0];
            transaction.source = "<script>alert('source')</script>".into();
            transaction.exchange = Some("NeedleExchange & venue".into());
            transaction.broker = Some("NeedleBroker <script>".into());
            transaction.notes =
                Some("First line\n<script>alert('note')</script>\nSecond line".into());
            Ok(())
        })
        .await
        .unwrap();
    fixture
}

fn transaction_sidebar(page: &str) -> &str {
    page.split("<aside class=\"transaction-sidebar\"")
        .nth(1)
        .unwrap()
        .split("</aside>")
        .next()
        .unwrap()
}

#[tokio::test]
async fn notes_are_visible_and_escaped_without_collapsible_table_details() {
    let fixture = fixture_with_details().await;
    let page = html(fixture.get("/transactions").await, StatusCode::OK).await;
    assert!(page.contains("<th scope=\"col\">Notes</th>"));
    assert!(page.contains("class=\"note-text\""));
    assert!(page.contains("First line\n"));
    assert!(page.contains("Second line"));
    assert!(page.contains("No notes"));
    assert!(!page.contains("transaction-details"));
    assert!(!page.contains("<script>alert('note')</script>"));
    assert!(!page.contains("NeedleBroker"));
    assert!(!page.contains("NeedleExchange"));
}

#[tokio::test]
async fn transaction_search_still_matches_broker_exchange_and_visible_notes() {
    let fixture = fixture_with_details().await;
    for search in ["NeedleBroker", "NeedleExchange", "Second"] {
        let page = html(
            fixture.get(&format!("/transactions?search={search}")).await,
            StatusCode::OK,
        )
        .await;
        assert!(page.contains("1 matching entries"));
    }
}

#[tokio::test]
async fn edit_sidebar_shows_saved_metadata_and_the_delete_action() {
    let fixture = fixture_with_details().await;
    let id = fixture.persisted().transactions[0].id;
    let page = html(
        fixture.get(&format!("/transactions/{id}/edit")).await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("class=\"editor-layout\""));
    let sidebar = transaction_sidebar(&page);
    assert!(sidebar.contains("Transaction details"));
    assert!(sidebar.contains(&format!("<dt>Entry</dt><dd>#{id}</dd>")));
    assert!(sidebar.contains("<dt>Source</dt>"));
    assert!(sidebar.contains("<dt>Exchange</dt>"));
    assert!(sidebar.contains("<dt>Broker</dt>"));
    assert!(
        sidebar.contains("NeedleExchange &amp; venue")
            || sidebar.contains("NeedleExchange &#38; venue")
    );
    assert!(
        sidebar.contains("NeedleBroker &lt;script&gt;")
            || sidebar.contains("NeedleBroker &#60;script&#62;")
    );
    assert!(!sidebar.contains("<script>alert('source')</script>"));
    assert!(sidebar.contains(&format!("action=\"/transactions/{id}/delete\"")));
    assert!(page.contains("<input name=\"exchange\""));
    assert!(page.contains("<input name=\"broker\""));
}

#[tokio::test]
async fn new_transactions_do_not_show_saved_record_details_or_delete_actions() {
    let fixture = TestApp::with_assets();
    let page = html(fixture.get("/transactions/new").await, StatusCode::OK).await;
    assert!(!page.contains("transaction-sidebar"));
    assert!(!page.contains("Delete transaction"));
    assert!(page.contains("transaction-notes"));
}
