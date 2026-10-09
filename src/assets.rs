use crate::{
    model::{Asset, AssetKind, AssetMetadataSource, Id, StoreData},
    store::Store,
};

#[derive(Debug, thiserror::Error)]
#[error(
    "USD must use the Cash asset type because it is the fiat conversion intermediary. Set the USD asset to Cash before saving or refreshing prices."
)]
pub(crate) struct InvalidUsdAsset;

pub(crate) fn validate_usd_asset(asset: &Asset) -> anyhow::Result<()> {
    if asset.symbol.eq_ignore_ascii_case("USD") && asset.kind != AssetKind::Fiat {
        return Err(InvalidUsdAsset.into());
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AssetInput {
    pub symbol: String,
    pub name: String,
    pub kind: AssetKind,
    pub yahoo_symbol: Option<String>,
    pub tradingview_symbol: Option<String>,
    pub valuation_currency: Option<String>,
}

pub fn save_asset(store: &mut Store, id: Option<Id>, input: AssetInput) -> anyhow::Result<Id> {
    let mut asset = normalized_asset(input, id.unwrap_or_default(), id.is_none());
    let index = id
        .map(|id| {
            store
                .data
                .assets
                .iter()
                .position(|asset| asset.id == id)
                .ok_or_else(|| anyhow::anyhow!("unknown asset"))
        })
        .transpose()?;
    validate_asset(
        &store.data,
        &asset,
        index.map(|index| &store.data.assets[index]),
    )?;
    if let Some(index) = index {
        store.data.assets[index] = asset;
        Ok(store.data.assets[index].id)
    } else {
        let id = store.data.allocate_id();
        asset.id = id;
        store.data.assets.push(asset);
        Ok(id)
    }
}

fn normalized_asset(input: AssetInput, id: Id, creating: bool) -> Asset {
    let symbol = if creating {
        input.symbol.trim().to_ascii_uppercase()
    } else {
        input.symbol.trim().to_string()
    };
    let name = input.name.trim();
    Asset {
        id,
        name: if name.is_empty() {
            symbol.clone()
        } else {
            name.into()
        },
        symbol,
        kind: input.kind,
        yahoo_symbol: clean(input.yahoo_symbol),
        tradingview_symbol: clean(input.tradingview_symbol),
        valuation_currency: clean(input.valuation_currency)
            .map(|currency| currency.to_ascii_uppercase()),
        metadata_source: AssetMetadataSource::User,
    }
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_asset(data: &StoreData, asset: &Asset, previous: Option<&Asset>) -> anyhow::Result<()> {
    anyhow::ensure!(!asset.symbol.is_empty(), "asset symbol is required");
    validate_symbol(&asset.symbol, "asset symbol", false)?;
    anyhow::ensure!(
        asset.name.chars().count() <= 200,
        "asset name must be at most 200 characters"
    );
    validate_identity(data, asset, previous)?;
    validate_usd_asset(asset)?;
    if previous.is_none() && data.config.base_currencies.contains(&asset.symbol) {
        anyhow::ensure!(
            matches!(asset.kind, AssetKind::Fiat | AssetKind::Crypto),
            "a configured base currency must have the Cash or Crypto type"
        );
    }
    validate_currency_edit(data, previous.unwrap_or(asset), asset)?;
    validate_providers(asset)
}

fn validate_identity(
    data: &StoreData,
    asset: &Asset,
    previous: Option<&Asset>,
) -> anyhow::Result<()> {
    if let Some(previous) = previous {
        anyhow::ensure!(
            asset.symbol == previous.symbol,
            "asset symbols cannot be changed after creation"
        );
    }
    let changed = previous.is_none_or(|old| old.kind != asset.kind);
    if changed {
        anyhow::ensure!(
            !data
                .assets
                .iter()
                .any(|other| other.id != asset.id
                    && other.symbol.eq_ignore_ascii_case(&asset.symbol)),
            "this symbol is already used by another asset"
        );
    }
    Ok(())
}

fn validate_providers(asset: &Asset) -> anyhow::Result<()> {
    if let Some(symbol) = &asset.yahoo_symbol {
        validate_symbol(symbol, "Yahoo symbol", false)?;
        anyhow::ensure!(
            asset.kind != AssetKind::Crypto || symbol.to_ascii_uppercase().ends_with("-USD"),
            "crypto Yahoo symbols must quote in USD, for example BTC-USD"
        );
    }
    if let Some(symbol) = &asset.tradingview_symbol {
        validate_symbol(symbol, "TradingView symbol", true)?;
        anyhow::ensure!(
            symbol
                .split_once(':')
                .is_some_and(|(exchange, ticker)| !exchange.is_empty() && !ticker.is_empty()),
            "use an exchange-qualified TradingView symbol, for example NASDAQ:AAPL"
        );
    }
    validate_intrinsic_valuation(asset)
}

fn validate_intrinsic_valuation(asset: &Asset) -> anyhow::Result<()> {
    if let Some(currency) = &asset.valuation_currency {
        validate_symbol(currency, "valuation currency", false)?;
        anyhow::ensure!(
            asset.kind.supports_intrinsic_valuation(),
            "intrinsic valuation is only available for property, liabilities, and custom assets"
        );
    }
    Ok(())
}

fn validate_symbol(symbol: &str, label: &str, allow_colon: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        symbol.len() <= 100
            && symbol
                .chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || ".-_=^/".contains(character)
                    || allow_colon && character == ':'),
        "{label} must contain only letters, numbers, or ticker punctuation, without spaces"
    );
    Ok(())
}

fn validate_currency_edit(data: &StoreData, previous: &Asset, asset: &Asset) -> anyhow::Result<()> {
    if currency_in_use(data, previous) {
        anyhow::ensure!(
            matches!(asset.kind, AssetKind::Fiat | AssetKind::Crypto),
            "an asset used as a currency must have the Cash or Crypto type"
        );
    }
    Ok(())
}

pub fn currency_in_use(data: &StoreData, asset: &Asset) -> bool {
    if !matches!(asset.kind, AssetKind::Fiat | AssetKind::Crypto) {
        return false;
    }
    if data.transactions.iter().any(|transaction| {
        transaction.quote_asset_id == Some(asset.id) || transaction.fee_asset_id == Some(asset.id)
    }) {
        return true;
    }
    data.config.base_currencies.contains(&asset.symbol)
        || data.config.default_base_currency == asset.symbol
        || data.config.selected_base_currency == asset.symbol
        || data
            .prices
            .iter()
            .any(|price| price.currency == asset.symbol)
        || data
            .assets
            .iter()
            .any(|other| other.valuation_currency.as_ref() == Some(&asset.symbol))
}
