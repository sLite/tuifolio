use crate::model::{AssetKind, StoreData};

// Currency identity does not require an actual cash holding. These are ISO
// currency identifiers, not arbitrary stock tickers or provider minor units.
const FIAT_CODES: &str = "AED AFN ALL AMD ANG AOA ARS AUD AWG AZN BAM BBD BDT BGN BHD BIF BMD BND BOB BOV BRL BSD BTN BWP BYN BZD CAD CDF CHE CHF CHW CLF CLP CNY COP COU CRC CUP CVE CZK DJF DKK DOP DZD EGP ERN ETB EUR FJD FKP GBP GEL GHS GIP GMD GNF GTQ GYD HKD HNL HTG HUF IDR ILS INR IQD IRR ISK JMD JOD JPY KES KGS KHR KMF KPW KRW KWD KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD MMK MNT MOP MRU MUR MVR MWK MXN MXV MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN PYG QAR RON RSD RUB RWF SAR SBD SCR SDG SEK SGD SHP SLE SLL SOS SRD SSP STN SVC SYP SZL THB TJS TMT TND TOP TRY TTD TWD TZS UAH UGX USD USN UYI UYU UYW UZS VED VES VND VUV WST XAF XCD XCG XOF XPF YER ZAR ZMW ZWG ZWL";

pub fn is_fiat_code(code: &str) -> bool {
    FIAT_CODES
        .split_whitespace()
        .any(|candidate| candidate == code)
}

pub fn normalize_reporting_currency(data: &StoreData, value: &str) -> anyhow::Result<String> {
    let code = value.trim().to_ascii_uppercase();
    anyhow::ensure!(!code.is_empty(), "reporting currency is required");
    let matches = data
        .assets
        .iter()
        .filter(|asset| {
            asset.symbol.eq_ignore_ascii_case(&code)
                && matches!(asset.kind, AssetKind::Fiat | AssetKind::Crypto)
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
