use serde::Deserialize;

use super::forms::enum_value;
use crate::{
    assets::AssetInput,
    model::{Asset, AssetKind},
};

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct AssetForm {
    pub symbol: String,
    pub name: String,
    pub kind: String,
    pub yahoo_symbol: String,
    pub tradingview_symbol: String,
    pub valuation_currency: String,
}

impl AssetForm {
    pub fn new() -> Self {
        Self {
            kind: "Stock".into(),
            ..Self::default()
        }
    }

    pub fn from_asset(asset: &Asset) -> Self {
        Self {
            symbol: asset.symbol.clone(),
            name: asset.name.clone(),
            kind: format!("{:?}", asset.kind),
            yahoo_symbol: asset.yahoo_symbol.clone().unwrap_or_default(),
            tradingview_symbol: asset.tradingview_symbol.clone().unwrap_or_default(),
            valuation_currency: asset.valuation_currency.clone().unwrap_or_default(),
        }
    }

    pub fn input(&self) -> anyhow::Result<AssetInput> {
        Ok(AssetInput {
            symbol: self.symbol.clone(),
            name: self.name.clone(),
            kind: enum_value(&self.kind, "asset type")?,
            yahoo_symbol: Some(self.yahoo_symbol.clone()),
            tradingview_symbol: Some(self.tradingview_symbol.clone()),
            valuation_currency: Some(self.valuation_currency.clone()),
        })
    }

    pub fn supports_intrinsic_valuation(&self) -> bool {
        enum_value::<AssetKind>(&self.kind, "asset type")
            .is_ok_and(AssetKind::supports_intrinsic_valuation)
    }
}
