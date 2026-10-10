use axum::http::StatusCode;
use rust_decimal_macros::dec;

use super::fixture::{TestApp, configure_asset, html};
use crate::{
    accounting::build_report,
    ledger::rebuild_ledger,
    model::{AssetKind, LedgerEffect, LedgerRole, TransactionKind},
};

const NON_QUOTED_KINDS: [&str; 6] = [
    "Gift",
    "StakingReward",
    "Deposit",
    "Withdraw",
    "LiabilityIncrease",
    "LiabilityDecrease",
];

const QUOTE_CASES: [(&str, bool, bool); 10] = [
    ("Buy", true, true),
    ("Sell", true, true),
    ("Gift", false, false),
    ("StakingReward", false, false),
    ("AssetIncrease", true, false),
    ("AssetDecrease", true, false),
    ("Deposit", false, false),
    ("Withdraw", false, false),
    ("LiabilityIncrease", false, false),
    ("LiabilityDecrease", false, false),
];

fn body(fixture: &TestApp, kind: &str) -> String {
    let data = fixture.persisted();
    let portfolio = data.portfolios[0].id;
    let asset = data.assets[0].id;
    format!(
        "portfolio_id={portfolio}&timestamp=2026-10-08T12%3A30&kind={kind}&base_asset_id={asset}&base_amount=1"
    )
}

#[tokio::test]
async fn cash_and_liability_forms_initially_hide_the_quote_section() {
    let fixture = TestApp::with_assets();
    let eur = fixture.asset_id("EUR").await;
    let debt = fixture
        .state
        .edit(|store| Ok(configure_asset(store, "DEBT", "Debt", AssetKind::Liability)))
        .await
        .unwrap();
    for asset in [eur, debt] {
        let page = html(
            fixture
                .get(&format!("/transactions/new?asset={asset}"))
                .await,
            StatusCode::OK,
        )
        .await;
        assert!(page.contains("data-transaction-quote hidden disabled"));
    }
    let btc = fixture.asset_id("BTC").await;
    let page = html(
        fixture.get(&format!("/transactions/new?asset={btc}")).await,
        StatusCode::OK,
    )
    .await;
    assert!(!page.contains("data-transaction-quote hidden disabled"));
}

#[tokio::test]
async fn quote_visibility_and_requirements_match_each_selected_transaction_kind() {
    let fixture = TestApp::with_assets();
    for (kind, supports, requires) in QUOTE_CASES {
        let body = body(&fixture, kind).replace("base_amount=1", "base_amount=-1")
            + "&quote_ledger_effect=Ignore";
        let page = html(
            fixture.post("/transactions/new", &body).await,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
        assert!(page.contains("quantity must be greater than zero"));
        assert_eq!(
            page.contains("data-transaction-quote hidden disabled"),
            !supports
        );
        let selected = format!(
            "value=\"{kind}\" selected data-supports-quote=\"{supports}\" data-requires-quote=\"{requires}\""
        );
        assert!(page.contains(&selected));
    }
}

#[tokio::test]
async fn non_quoted_types_save_without_hidden_fields_and_still_post_fees() {
    let fixture = TestApp::with_assets();
    let eur = fixture.asset_id("EUR").await;
    for kind in NON_QUOTED_KINDS {
        let body = body(&fixture, kind) + &format!("&fee_asset_id={eur}&fee_amount=2");
        assert_eq!(
            fixture.post("/transactions/new", &body).await.status(),
            StatusCode::SEE_OTHER
        );
        let data = fixture.persisted();
        let transaction = data.transactions.last().unwrap();
        assert_eq!(transaction.quote_asset_id, None);
        assert_eq!(transaction.quote_amount, None);
        assert_eq!(transaction.quote_ledger_effect, LedgerEffect::Ignore);
        let fee = data
            .ledger_entries
            .iter()
            .find(|entry| {
                entry.transaction_id == transaction.id && matches!(entry.role, LedgerRole::Fee)
            })
            .unwrap();
        assert_eq!(fee.asset_id, eur);
        assert_eq!(fee.quantity_delta, dec!(-2));
    }
}

#[tokio::test]
async fn forged_quote_fields_are_rejected_for_non_quoted_types() {
    let fixture = TestApp::with_assets();
    let eur = fixture.asset_id("EUR").await;
    let original = serde_json::to_value(fixture.persisted()).unwrap();
    let quotes = [
        format!("&quote_asset_id={eur}"),
        format!("&quote_asset_id={eur}&quote_amount=0"),
        "&quote_amount=100".into(),
    ];
    // Staking rewards separately preserve supported deposit quote-asset metadata.
    // They reject quote payments, covered by staking_rewards tests.
    for kind in NON_QUOTED_KINDS
        .into_iter()
        .filter(|kind| *kind != "StakingReward")
    {
        for quote in &quotes {
            let body = body(&fixture, kind) + quote;
            let page = html(
                fixture.post("/transactions/new", &body).await,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
            assert!(page.contains("quote fields are only applicable"));
            assert!(page.contains("data-transaction-quote hidden disabled"));
            assert_eq!(serde_json::to_value(fixture.persisted()).unwrap(), original);
        }
    }
}

#[tokio::test]
async fn changing_a_trade_to_deposit_clears_omitted_quote_fields() {
    let fixture = TestApp::with_assets();
    fixture
        .post("/transactions/new", &fixture.buy_body().await)
        .await;
    let previous = fixture.persisted().transactions[0].clone();
    let path = format!("/transactions/{}/edit", previous.id);
    assert_eq!(
        fixture
            .submit_edit(&path, &body(&fixture, "Deposit"))
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let data = fixture.persisted();
    let updated = &data.transactions[0];
    assert_eq!(updated.id, previous.id);
    assert_eq!(updated.source_row_hash, previous.source_row_hash);
    assert_eq!(updated.kind, TransactionKind::Deposit);
    assert_eq!(updated.quote_asset_id, None);
    assert_eq!(updated.quote_amount, None);
    assert_eq!(build_report(&data).holdings[0].net_invested, None);
}

#[tokio::test]
async fn historical_unsupported_quotes_have_no_cash_effect_label_or_pnl_basis() {
    let fixture = TestApp::new(true);
    fixture
        .state
        .edit(|store| {
            store.data.transactions[0].kind = TransactionKind::Deposit;
            store.data.transactions[0].quote_ledger_effect = LedgerEffect::Post;
            rebuild_ledger(&mut store.data)
        })
        .await
        .unwrap();
    let id = fixture.persisted().transactions[0].id;
    let page = html(fixture.get("/transactions").await, StatusCode::OK).await;
    assert!(!page.contains("Cash posted"));
    assert!(!page.contains("Cost basis only"));
    let page = html(
        fixture.get(&format!("/transactions/{id}/edit")).await,
        StatusCode::OK,
    )
    .await;
    assert!(page.contains("data-transaction-quote hidden disabled"));
    let report = build_report(&fixture.persisted());
    let btc = report
        .holdings
        .iter()
        .find(|holding| holding.symbol == "BTC")
        .unwrap();
    assert_eq!(btc.net_invested, None);
    assert_eq!(btc.unrealized_pnl, None);
}

#[tokio::test]
async fn supported_types_still_require_an_explicit_cash_effect() {
    let fixture = TestApp::with_assets();
    let eur = fixture.asset_id("EUR").await;
    let body = body(&fixture, "Buy") + &format!("&quote_asset_id={eur}&quote_amount=100");
    let page = html(
        fixture.post("/transactions/new", &body).await,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert!(page.contains("select a valid cash effect"));
    assert!(fixture.persisted().transactions.is_empty());
}
