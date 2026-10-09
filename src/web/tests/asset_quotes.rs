use axum::http::StatusCode;
use rust_decimal_macros::dec;

use super::fixture::{TestApp, html};
use crate::{
    accounting::build_report,
    assets::{AssetInput, save_asset},
    model::{
        AssetKind, Id, LedgerEffect,
        LedgerEffect::{Ignore, Post},
        TransactionKind,
        TransactionKind::{AssetDecrease, AssetIncrease},
    },
};

struct PropertyFixture {
    app: TestApp,
    asset_id: Id,
}

impl PropertyFixture {
    async fn new() -> Self {
        let app = TestApp::with_assets();
        let asset_id = app
            .state
            .edit(|store| {
                save_asset(
                    store,
                    None,
                    AssetInput {
                        symbol: "HOME".into(),
                        name: "Home".into(),
                        kind: AssetKind::Property,
                        yahoo_symbol: None,
                        tradingview_symbol: None,
                        valuation_currency: Some("EUR".into()),
                    },
                )
            })
            .await
            .unwrap();
        Self { app, asset_id }
    }

    fn body(
        &self,
        kind: TransactionKind,
        quantity: i64,
        amount: &str,
        effect: LedgerEffect,
    ) -> String {
        let data = self.app.persisted();
        let portfolio = data.portfolios[0].id;
        let quote = data
            .assets
            .iter()
            .find(|asset| asset.symbol == "EUR")
            .unwrap()
            .id;
        format!(
            "portfolio_id={portfolio}&timestamp=2026-10-08T21%3A00&kind={kind:?}&base_asset_id={}&base_amount={quantity}&quote_asset_id={quote}&quote_amount={amount}&quote_ledger_effect={effect:?}",
            self.asset_id
        )
    }

    fn assert_zero_basis(&self) {
        let data = self.app.persisted();
        assert_eq!(data.transactions[0].quote_amount, Some(dec!(0)));
        assert_eq!(data.transactions[0].base_amount, dec!(350000));
        assert_eq!(data.ledger_entries.len(), 1);
        assert_eq!(data.ledger_entries[0].asset_id, self.asset_id);
        let report = build_report(&data);
        assert_eq!(report.holdings[0].net_invested, Some(dec!(0)));
        assert_eq!(report.holdings[0].unrealized_pnl, Some(dec!(350000)));
    }

    fn cash_balance(&self) -> rust_decimal::Decimal {
        let data = self.app.persisted();
        let cash = data
            .assets
            .iter()
            .find(|asset| asset.symbol == "EUR")
            .unwrap()
            .id;
        data.ledger_entries
            .iter()
            .filter(|entry| entry.asset_id == cash)
            .map(|entry| entry.quantity_delta)
            .sum()
    }

    fn assert_pnl(&self, value: i64, invested: i64, pnl: i64) {
        let report = build_report(&self.app.persisted());
        let holding = report
            .holdings
            .iter()
            .find(|holding| holding.asset_id == self.asset_id)
            .unwrap();
        assert_eq!(holding.value, Some(value.into()));
        assert_eq!(holding.net_invested, Some(invested.into()));
        assert_eq!(holding.unrealized_pnl, Some(pnl.into()));
    }
}

#[tokio::test]
async fn creating_a_gifted_property_saves_zero_basis_and_prefills_it_in_the_editor() {
    let fixture = PropertyFixture::new().await;
    let response = fixture
        .app
        .post(
            "/transactions/new",
            &fixture.body(AssetIncrease, 350000, "0", Ignore),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    fixture.assert_zero_basis();
    let id = fixture.app.persisted().transactions[0].id;
    let page = html(
        fixture.app.get(&format!("/transactions/{id}/edit")).await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("name=\"quote_amount\" value=\"0\""));
    assert!(page.contains("Enter 0 for a known zero cost basis"));
}

#[tokio::test]
async fn editing_unknown_basis_to_zero_preserves_the_transaction_and_creates_no_cash_entry() {
    let fixture = PropertyFixture::new().await;
    fixture
        .app
        .post(
            "/transactions/new",
            &fixture.body(AssetIncrease, 350000, "", Ignore),
        )
        .await;
    let previous = fixture.app.persisted().transactions[0].clone();
    assert_eq!(
        build_report(&fixture.app.persisted()).holdings[0].net_invested,
        None
    );
    let path = format!("/transactions/{}/edit", previous.id);
    for amount in ["0", "0.00"] {
        let body = fixture.body(AssetIncrease, 350000, amount, Ignore);
        let response = fixture.app.submit_edit(&path, &body).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        fixture.assert_zero_basis();
        let updated = &fixture.app.persisted().transactions[0];
        assert_eq!(updated.id, previous.id);
        assert_eq!(updated.source, previous.source);
        assert_eq!(updated.source_row_hash, previous.source_row_hash);
    }
}

#[tokio::test]
async fn invalid_gift_amounts_preserve_input_and_do_not_save() {
    let fixture = PropertyFixture::new().await;
    let before = serde_json::to_value(fixture.app.persisted()).unwrap();
    let page = html(
        fixture
            .app
            .post(
                "/transactions/new",
                &fixture.body(AssetIncrease, 350000, "-1", Ignore),
            )
            .await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("quote amount cannot be negative"));
    assert!(page.contains("name=\"quote_amount\" value=\"-1\""));
    let body = fixture.body(AssetIncrease, 0, "0", Ignore);
    let page = html(
        fixture.app.post("/transactions/new", &body).await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("quantity must be greater than zero"));
    assert_eq!(
        serde_json::to_value(fixture.app.persisted()).unwrap(),
        before
    );
}

#[tokio::test]
async fn property_costs_and_valuation_adjustments_render_correct_pnl() {
    let fixture = PropertyFixture::new().await;
    for (quantity, amount) in [(255000, "420000"), (245000, "")] {
        let body = fixture.body(AssetIncrease, quantity, amount, Ignore);
        assert_eq!(
            fixture.app.post("/transactions/new", &body).await.status(),
            StatusCode::SEE_OTHER
        );
    }
    fixture.assert_pnl(500000, 420000, 80000);
    assert_eq!(fixture.cash_balance(), dec!(0));
    let page = html(fixture.app.get("/").await, StatusCode::OK).await;
    assert!(page.contains("500,000.00"));
    assert!(page.contains("420,000.00"));
    assert!(page.contains("80,000.00"));
}

#[tokio::test]
async fn posted_asset_increase_quotes_deduct_cash_and_persist_the_cost_basis() {
    let fixture = PropertyFixture::new().await;
    let body = fixture.body(AssetIncrease, 500000, "420000", Post);
    assert_eq!(
        fixture.app.post("/transactions/new", &body).await.status(),
        StatusCode::SEE_OTHER
    );
    fixture.assert_pnl(500000, 420000, 80000);
    assert_eq!(fixture.cash_balance(), dec!(-420000));
    assert_eq!(fixture.app.persisted().ledger_entries.len(), 2);
    let page = html(fixture.app.get("/transactions").await, StatusCode::OK).await;
    assert!(page.contains("Cash posted"));
}

#[tokio::test]
async fn posted_asset_decrease_quotes_add_cash_and_subtract_proceeds_from_net_invested() {
    let fixture = PropertyFixture::new().await;
    let purchase = fixture.body(AssetIncrease, 500000, "420000", Ignore);
    fixture.app.post("/transactions/new", &purchase).await;
    let disposal = fixture.body(AssetDecrease, 100000, "60000", Post);
    assert_eq!(
        fixture
            .app
            .post("/transactions/new", &disposal)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    fixture.assert_pnl(400000, 360000, 40000);
    assert_eq!(fixture.cash_balance(), dec!(60000));
    assert_eq!(fixture.app.persisted().ledger_entries.len(), 3);
}

#[tokio::test]
async fn editing_the_asset_quote_cash_effect_updates_cash_and_preserves_the_record() {
    let fixture = PropertyFixture::new().await;
    let body = fixture.body(AssetIncrease, 500000, "420000", Ignore);
    fixture.app.post("/transactions/new", &body).await;
    let previous = fixture.app.persisted().transactions[0].clone();
    let path = format!("/transactions/{}/edit", previous.id);
    let body = fixture.body(AssetIncrease, 500000, "420000", Post);
    assert_eq!(
        fixture.app.submit_edit(&path, &body).await.status(),
        StatusCode::SEE_OTHER
    );
    let updated = &fixture.app.persisted().transactions[0];
    assert_eq!(updated.id, previous.id);
    assert_eq!(updated.source_row_hash, previous.source_row_hash);
    assert_eq!(updated.quote_ledger_effect, Post);
    fixture.assert_pnl(500000, 420000, 80000);
    assert_eq!(fixture.cash_balance(), dec!(-420000));
    let page = html(fixture.app.get(&path).await, StatusCode::OK).await;
    assert!(page.contains("value=\"Post\" selected"));
}
