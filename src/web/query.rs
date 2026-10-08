use serde::{Deserialize, Deserializer};

use super::error::WebError;
use crate::model::{Id, StoreData, Transaction};

pub(super) const PAGE_SIZE: usize = 50;

#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum AssetScope {
    #[default]
    Primary,
    Any,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct PageQuery {
    #[serde(deserialize_with = "optional_id")]
    pub portfolio: Option<Id>,
    #[serde(deserialize_with = "optional_id")]
    pub asset: Option<Id>,
    pub scope: AssetScope,
    pub search: String,
    pub page: usize,
    pub notice: String,
}

impl PageQuery {
    pub fn validate(&self, data: &StoreData) -> Result<(), WebError> {
        if self
            .portfolio
            .is_some_and(|id| !data.portfolios.iter().any(|p| p.id == id))
            || self
                .asset
                .is_some_and(|id| !data.assets.iter().any(|a| a.id == id))
        {
            return Err(WebError::not_found());
        }
        Ok(())
    }

    pub fn matches(&self, transaction: &Transaction) -> bool {
        self.portfolio
            .is_none_or(|id| transaction.portfolio_id == id)
            && self.asset.is_none_or(|id| {
                transaction.base_asset_id == id
                    || self.scope == AssetScope::Any
                        && (transaction.quote_asset_id == Some(id)
                            || transaction.fee_asset_id == Some(id))
            })
    }

    pub fn url(&self, page: usize) -> String {
        format!(
            "/transactions?portfolio={}&asset={}&scope={}&search={}&page={page}",
            self.portfolio.map(|id| id.to_string()).unwrap_or_default(),
            self.asset.map(|id| id.to_string()).unwrap_or_default(),
            if self.scope == AssetScope::Any {
                "any"
            } else {
                "primary"
            },
            urlencoding::encode(&self.search)
        )
    }
}

fn optional_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Id>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .filter(|value| !value.is_empty())
        .map(|value| value.parse().map_err(serde::de::Error::custom))
        .transpose()
}
