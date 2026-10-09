use axum::http::StatusCode;
use rust_decimal_macros::dec;

use super::fixture::{TestApp, html};
use crate::{
    accounting::build_report,
    assets::{AssetInput, save_asset},
    model::{AssetKind, Id, LedgerEffect, TransactionKind},
};

struct Fixture {
    app: TestApp,
    asset: Id,
}

impl Fixture {
    async fn new() -> Self {
        let app = TestApp::with_assets();
        let asset = app
            .state
            .edit(|store| {
                save_asset(
                    store,
                    None,
                    AssetInput {
                        symbol: "HOME".into(),
                        name: "Gifted home".into(),
                        kind: AssetKind::Property,
                        yahoo_symbol: None,
                        tradingview_symbol: None,
                        valuation_currency: Some("EUR".into()),
                    },
                )
            })
            .await
            .unwrap();
        Self { app, asset }
    }

    fn body(&self, kind: &str) -> String {
        let portfolio = self.app.persisted().portfolios[0].id;
        format!(
            "portfolio_id={portfolio}&timestamp=2026-10-08T21%3A00&kind={kind}&base_asset_id={}&base_amount=350000",
            self.asset
        )
    }

    fn assert_zero_basis(&self) {
        let data = self.app.persisted();
        let transaction = &data.transactions[0];
        assert_eq!(transaction.kind, TransactionKind::Gift);
        assert_eq!(transaction.quote_asset_id, None);
        assert_eq!(transaction.quote_amount, None);
        assert_eq!(transaction.quote_ledger_effect, LedgerEffect::Ignore);
        let report = build_report(&data);
        let holding = report
            .holdings
            .iter()
            .find(|holding| holding.asset_id == self.asset)
            .unwrap();
        assert_eq!(holding.net_invested, Some(dec!(0)));
        assert_eq!(holding.unrealized_pnl, Some(dec!(350000)));
        assert_eq!(report.portfolios[0].unrealized_pnl, dec!(350000));
    }
}

#[tokio::test]
async fn gifts_save_without_quote_fields_and_render_their_implicit_zero_cost() {
    let fixture = Fixture::new().await;
    assert_eq!(
        fixture
            .app
            .post("/transactions/new", &fixture.body("Gift"))
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    fixture.assert_zero_basis();
    assert_eq!(fixture.app.persisted().ledger_entries.len(), 1);
    let page = html(fixture.app.get("/transactions").await, StatusCode::OK).await;
    assert!(page.contains(">Gift</span>"));
    assert!(page.contains("0<span class=\"cell-subtitle\">Zero cost basis</span>"));
    assert!(!page.contains("Cash posted"));
    let id = fixture.app.persisted().transactions[0].id;
    let page = html(
        fixture.app.get(&format!("/transactions/{id}/edit")).await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("value=\"Gift\" selected"));
    assert!(page.contains("data-transaction-quote hidden disabled"));
    assert!(!page.contains("data-gift-help hidden"));
}

#[tokio::test]
async fn changing_a_paid_transaction_to_gift_clears_quotes_and_preserves_fees_and_notes() {
    let fixture = Fixture::new().await;
    let cash = fixture.app.asset_id("EUR").await;
    let extras = format!("&fee_asset_id={cash}&fee_amount=2&notes=Keep%20this%20note");
    let paid = fixture.body("Buy")
        + &format!("&quote_asset_id={cash}&quote_amount=100000&quote_ledger_effect=Post")
        + &extras;
    fixture.app.post("/transactions/new", &paid).await;
    let previous = fixture.app.persisted().transactions[0].clone();
    let path = format!("/transactions/{}/edit", previous.id);
    let gift = fixture.body("Gift") + &extras;
    assert_eq!(
        fixture.app.post(&path, &gift).await.status(),
        StatusCode::SEE_OTHER
    );
    fixture.assert_zero_basis();
    let data = fixture.app.persisted();
    let updated = &data.transactions[0];
    assert_eq!(updated.id, previous.id);
    assert_eq!(updated.source_row_hash, previous.source_row_hash);
    assert_eq!(updated.fee_amount, Some(dec!(2)));
    assert_eq!(updated.notes.as_deref(), Some("Keep this note"));
    assert_eq!(data.ledger_entries.len(), 2);
    assert_eq!(data.ledger_entries[1].asset_id, cash);
    assert_eq!(data.ledger_entries[1].quantity_delta, dec!(-2));
}

#[tokio::test]
async fn no_op_gift_edits_preserve_the_record_and_implicit_cost_basis() {
    let fixture = Fixture::new().await;
    let body = fixture.body("Gift");
    fixture.app.post("/transactions/new", &body).await;
    let original = serde_json::to_value(fixture.app.persisted()).unwrap();
    let id = fixture.app.persisted().transactions[0].id;
    assert_eq!(
        fixture
            .app
            .post(&format!("/transactions/{id}/edit"), &body)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        serde_json::to_value(fixture.app.persisted()).unwrap(),
        original
    );
    fixture.assert_zero_basis();
}

#[tokio::test]
async fn changing_gift_to_an_unquoted_asset_increase_restores_unknown_basis() {
    let fixture = Fixture::new().await;
    fixture
        .app
        .post("/transactions/new", &fixture.body("Gift"))
        .await;
    let id = fixture.app.persisted().transactions[0].id;
    let body = fixture.body("AssetIncrease") + "&quote_ledger_effect=Ignore";
    assert_eq!(
        fixture
            .app
            .post(&format!("/transactions/{id}/edit"), &body)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let report = build_report(&fixture.app.persisted());
    assert_eq!(report.holdings[0].net_invested, None);
    assert_eq!(report.holdings[0].unrealized_pnl, None);
}

#[tokio::test]
async fn forged_gift_quotes_are_rejected_and_do_not_write() {
    let fixture = Fixture::new().await;
    let cash = fixture.app.asset_id("EUR").await;
    let before = serde_json::to_value(fixture.app.persisted()).unwrap();
    for amount in ["0", "100"] {
        let body = fixture.body("Gift") + &format!("&quote_asset_id={cash}&quote_amount={amount}");
        let page = html(
            fixture.app.post("/transactions/new", &body).await,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
        assert!(page.contains("quote fields are only applicable"));
        assert!(page.contains("data-transaction-quote hidden disabled"));
        assert_eq!(
            serde_json::to_value(fixture.app.persisted()).unwrap(),
            before
        );
    }
}
