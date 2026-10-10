use crate::model::{AssetKind, StoreData};

// Currency identity does not require an actual cash holding. These are ISO
// currency identifiers, not arbitrary stock tickers or provider minor units.
const FIAT_CODES: &str = "AED AFN ALL AMD ANG AOA ARS AUD AWG AZN BAM BBD BDT BGN BHD BIF BMD BND BOB BOV BRL BSD BTN BWP BYN BZD CAD CDF CHE CHF CHW CLF CLP CNY COP COU CRC CUP CVE CZK DJF DKK DOP DZD EGP ERN ETB EUR FJD FKP GBP GEL GHS GIP GMD GNF GTQ GYD HKD HNL HTG HUF IDR ILS INR IQD IRR ISK JMD JOD JPY KES KGS KHR KMF KPW KRW KWD KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD MMK MNT MOP MRU MUR MVR MWK MXN MXV MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN PYG QAR RON RSD RUB RWF SAR SBD SCR SDG SEK SGD SHP SLE SLL SOS SRD SSP STN SVC SYP SZL THB TJS TMT TND TOP TRY TTD TWD TZS UAH UGX USD USN UYI UYU UYW UZS VED VES VND VUV WST XAF XCD XCG XOF XPF YER ZAR ZMW ZWG ZWL";

pub(crate) fn quote_unit(code: &str) -> (String, rust_decimal::Decimal) {
    let code = code.trim();
    if code == "GBp" || code.eq_ignore_ascii_case("GBX") {
        ("GBP".into(), rust_decimal::Decimal::new(1, 2))
    } else {
        (code.to_ascii_uppercase(), rust_decimal::Decimal::ONE)
    }
}

pub fn is_fiat_code(code: &str) -> bool {
    FIAT_CODES
        .split_whitespace()
        .any(|candidate| candidate == code)
}

/// Resolve a money unit, not a display ticker. ISO fiat units cannot be
/// identified with a crypto asset that happens to share their label.
pub(crate) fn currency_asset<'a>(
    data: &'a StoreData,
    code: &str,
) -> Option<&'a crate::model::Asset> {
    currency_asset_from(data.assets.iter(), code)
}

pub(crate) fn currency_asset_from<'a>(
    assets: impl Iterator<Item = &'a crate::model::Asset>,
    code: &str,
) -> Option<&'a crate::model::Asset> {
    let code = code.trim().to_ascii_uppercase();
    let mut candidates = assets.filter(|asset| {
        asset.symbol.eq_ignore_ascii_case(&code)
            && if is_fiat_code(&quote_unit(&code).0) {
                asset.kind == AssetKind::Fiat
            } else {
                matches!(asset.kind, AssetKind::Fiat | AssetKind::Crypto)
            }
    });
    let asset = candidates.next()?;
    candidates.next().is_none().then_some(asset)
}

pub fn normalize_reporting_currency(data: &StoreData, value: &str) -> anyhow::Result<String> {
    let code = value.trim().to_ascii_uppercase();
    anyhow::ensure!(!code.is_empty(), "reporting currency is required");
    let matches = data
        .assets
        .iter()
        .filter(|asset| {
            asset.symbol.eq_ignore_ascii_case(&code)
                && if is_fiat_code(&quote_unit(&code).0) {
                    asset.kind == AssetKind::Fiat
                } else {
                    matches!(asset.kind, AssetKind::Fiat | AssetKind::Crypto)
                }
        })
        .count();
    anyhow::ensure!(
        matches <= 1,
        "reporting currency {code} has ambiguous asset identities"
    );
    anyhow::ensure!(
        is_fiat_code(&code) || matches == 1 || matches!(code.as_str(), "BTC" | "ETH"),
        "unsupported reporting currency {code}; choose a fiat currency or a unique Cash/Crypto asset"
    );
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AssetMetadataSource};

    #[test]
    fn normalizes_supported_currencies_without_requiring_cash_holdings() {
        let data = StoreData::default();
        for (input, code) in [(" eur ", "EUR"), ("gbp", "GBP"), ("BTC", "BTC")] {
            assert_eq!(normalize_reporting_currency(&data, input).unwrap(), code);
        }
        for input in ["", "AAPL", "NOTACURRENCY"] {
            assert!(normalize_reporting_currency(&data, input).is_err());
        }
    }

    #[test]
    fn fiat_reference_ignores_a_crypto_ticker_collision() {
        let mut data = StoreData::default();
        let coin = Asset {
            id: 1,
            symbol: "GBP".into(),
            name: "Coin".into(),
            kind: AssetKind::Crypto,
            yahoo_symbol: None,
            tradingview_symbol: None,
            valuation_currency: None,
            metadata_source: AssetMetadataSource::User,
        };
        data.assets.push(coin.clone());
        assert!(currency_asset(&data, "GBP").is_none());
        let mut cash = coin;
        cash.id = 2;
        cash.kind = AssetKind::Fiat;
        data.assets.push(cash);
        assert_eq!(normalize_reporting_currency(&data, "GBP").unwrap(), "GBP");
        assert_eq!(currency_asset(&data, "GBP").unwrap().id, 2);
    }

    #[test]
    fn allows_unique_currency_assets_not_stock_tickers_and_rejects_ambiguity() {
        let mut data = StoreData::default();
        let asset = Asset {
            id: 1,
            symbol: "COIN".into(),
            name: "Coin".into(),
            kind: AssetKind::Stock,
            yahoo_symbol: None,
            tradingview_symbol: None,
            valuation_currency: None,
            metadata_source: AssetMetadataSource::User,
        };
        data.assets.push(asset);
        assert!(normalize_reporting_currency(&data, "COIN").is_err());
        data.assets[0].kind = AssetKind::Crypto;
        assert_eq!(normalize_reporting_currency(&data, "coin").unwrap(), "COIN");
        let mut duplicate = data.assets[0].clone();
        duplicate.id = 2;
        data.assets.push(duplicate);
        assert!(
            normalize_reporting_currency(&data, "COIN")
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
    }
}
