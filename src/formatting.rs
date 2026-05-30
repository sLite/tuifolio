use rust_decimal::Decimal;

pub fn money(value: Decimal) -> String {
    format!("{:.2}", value.round_dp(2))
}

pub fn quantity(value: Decimal) -> String {
    value.round_dp(6).to_string()
}
