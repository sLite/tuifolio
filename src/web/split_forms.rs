use serde::Deserialize;

use crate::model::{Id, StockSplit, StoreData};

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct SplitForm {
    pub effective_date: String,
    pub numerator: String,
    pub denominator: String,
    pub original_date: String,
    pub original_numerator: String,
    pub original_denominator: String,
}

impl SplitForm {
    pub fn from_split(split: &StockSplit) -> Self {
        Self {
            effective_date: split.effective_date.clone(),
            numerator: split.numerator.normalize().to_string(),
            denominator: split.denominator.normalize().to_string(),
            original_date: split.effective_date.clone(),
            original_numerator: split.numerator.to_string(),
            original_denominator: split.denominator.to_string(),
        }
    }

    pub fn input(&self, asset_id: Id) -> anyhow::Result<StockSplit> {
        parse_split(
            asset_id,
            &self.effective_date,
            &self.numerator,
            &self.denominator,
        )
    }

    pub fn previous(&self, asset_id: Id) -> anyhow::Result<StockSplit> {
        parse_split(
            asset_id,
            &self.original_date,
            &self.original_numerator,
            &self.original_denominator,
        )
    }

    pub fn is_edit(&self) -> bool {
        !self.original_date.is_empty()
    }
}

fn parse_split(
    asset_id: Id,
    date: &str,
    numerator: &str,
    denominator: &str,
) -> anyhow::Result<StockSplit> {
    Ok(StockSplit {
        asset_id,
        effective_date: date.trim().into(),
        numerator: numerator
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("new shares must be a decimal number"))?,
        denominator: denominator
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("old shares must be a decimal number"))?,
    })
}

pub(super) fn split_forms(data: &StoreData, id: Option<Id>) -> Vec<SplitForm> {
    let mut splits = data
        .config
        .stock_splits
        .iter()
        .filter(|split| Some(split.asset_id) == id)
        .collect::<Vec<_>>();
    splits.sort_by(|a, b| b.effective_date.cmp(&a.effective_date));
    splits.into_iter().map(SplitForm::from_split).collect()
}
