use askama::Template;
use axum::http::StatusCode;

use super::fixture::{TestApp, html};

#[derive(Template)]
#[template(
    source = "{% import \"privacy.html\" as privacy %}{% call privacy::amount(value) %}",
    ext = "html"
)]
struct PrivateAmount<'a> {
    value: &'a str,
}

#[test]
fn masks_use_the_same_placeholder_for_all_amount_sizes_and_signs() {
    let mask =
        "<span class=\"private-mask\" role=\"img\" aria-label=\"Hidden amount\">••••••</span>";
    for value in ["0", "0.00000001", "-1.00", "1,234,567,890.12", "250 EUR"] {
        let rendered = PrivateAmount { value }.render().unwrap();
        assert!(rendered.contains(&format!("<span class=\"private-value\">{value}</span>")));
        assert_eq!(rendered.matches(mask).count(), 1);
    }
    for value in ["", "Unavailable"] {
        assert_eq!(PrivateAmount { value }.render().unwrap(), value);
    }
}

#[tokio::test]
async fn reports_and_prices_wrap_all_displayed_financial_values() {
    let fixture = TestApp::new(true);
    let data = fixture.persisted();
    let portfolio = data.portfolios[0].id;
    let btc = fixture.asset_id("BTC").await;
    for (path, count) in [
        ("/".into(), 10),
        ("/portfolios".into(), 4),
        (format!("/portfolios/{portfolio}"), 10),
        ("/transactions".into(), 4),
        ("/assets".into(), 3),
        (format!("/assets/{btc}"), 1),
    ] {
        let page = html(fixture.get(&path).await, StatusCode::OK).await;
        assert_eq!(
            page.matches("class=\"private-value\"").count(),
            count,
            "{path}"
        );
        assert!(page.contains("data-privacy-toggle"));
        assert!(page.contains("<script src=\"/static/privacy.js\"></script>"));
    }
}

#[tokio::test]
async fn editor_amounts_are_maskable_without_changing_their_form_values() {
    let fixture = TestApp::new(true);
    let data = fixture.persisted();
    let id = data.transactions[0].id;
    let page = html(
        fixture.get(&format!("/transactions/{id}/edit")).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(page.matches("class=\"private-input\"").count(), 3);
    for (name, value) in [
        ("base_amount", "0.1"),
        ("quote_amount", "123.123456789"),
        ("fee_amount", "2"),
    ] {
        assert!(page.contains(&format!(
            "<span class=\"private-input\"><input name=\"{name}\" value=\"{value}\""
        )));
    }
    let usd = fixture.asset_id("USD").await;
    let page = html(fixture.get(&format!("/assets/{usd}")).await, StatusCode::OK).await;
    assert!(page.contains("<span class=\"private-input\"><input name=\"price\""));
}
