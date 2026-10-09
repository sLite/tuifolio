use std::collections::BTreeMap;

use super::{
    asset_forms::AssetForm,
    forms::PriceForm,
    query::PageQuery,
    split_forms::{SplitForm, split_forms},
    tables::{PriceView, asset_kind, chart_url, prices},
    views::{ASSET_KINDS, Choice, Common, enum_choices},
};
use crate::{
    assets::currency_in_use,
    model::{Asset, AssetKind, Id, StoreData},
};
use askama::Template;

pub(super) struct AssetRow {
    pub id: Id,
    pub symbol: String,
    pub name: String,
    pub kind: &'static str,
    pub yahoo_symbol: String,
    pub tradingview_symbol: String,
    pub pricing: &'static str,
    pub quotes: Vec<PriceView>,
}

impl AssetRow {
    fn new(asset: &Asset, quotes: Vec<PriceView>) -> Self {
        Self {
            id: asset.id,
            symbol: asset.symbol.clone(),
            name: asset.name.clone(),
            kind: asset_kind(asset.kind),
            yahoo_symbol: AssetForm::from_asset(asset).yahoo_symbol,
            tradingview_symbol: asset.tradingview_symbol.clone().unwrap_or_default(),
            pricing: pricing_method(asset),
            quotes,
        }
    }
}

fn pricing_method(asset: &Asset) -> &'static str {
    if asset.valuation_currency.is_some() {
        "Intrinsic value"
    } else if asset.kind == AssetKind::Fiat {
        "Exchange rates"
    } else if asset.yahoo_quote_symbol().is_some() {
        "Yahoo"
    } else {
        "Manual prices"
    }
}

fn quote_map(data: &StoreData) -> BTreeMap<Id, Vec<PriceView>> {
    let mut quotes = BTreeMap::<Id, Vec<PriceView>>::new();
    for price in prices(data) {
        quotes.entry(price.asset_id).or_default().push(price);
    }
    quotes
}

fn asset_rows(data: &StoreData, search: &str) -> Vec<AssetRow> {
    let mut quotes = quote_map(data);
    let mut assets = data.assets.iter().collect::<Vec<_>>();
    assets.sort_by(|a, b| a.symbol.cmp(&b.symbol).then(a.id.cmp(&b.id)));
    let search = search.to_lowercase();
    assets
        .into_iter()
        .filter(|asset| {
            format!(
                "{} {} {} {} {}",
                asset.symbol,
                asset.name,
                asset_kind(asset.kind),
                asset.yahoo_symbol.as_deref().unwrap_or_default(),
                asset.tradingview_symbol.as_deref().unwrap_or_default()
            )
            .to_lowercase()
            .contains(&search)
        })
        .map(|asset| AssetRow::new(asset, quotes.remove(&asset.id).unwrap_or_default()))
        .collect()
}

#[derive(Template)]
#[template(path = "assets.html")]
pub(super) struct AssetsPage {
    pub common: Common,
    pub rows: Vec<AssetRow>,
    pub search: String,
    pub error: String,
    pub notice: String,
    pub refreshing: bool,
}

impl AssetsPage {
    pub fn new(data: &StoreData, query: &PageQuery, feedback: Feedback, refreshing: bool) -> Self {
        Self {
            common: Common::new(data, "Assets", "assets", "/assets".into()),
            rows: asset_rows(data, &query.search),
            search: query.search.clone(),
            error: feedback.error,
            notice: feedback.notice,
            refreshing,
        }
    }
}

#[derive(Default)]
pub(super) struct Feedback {
    pub error: String,
    pub notice: String,
}

impl Feedback {
    pub fn error(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            ..Self::default()
        }
    }

    pub fn notice(notice: impl Into<String>) -> Self {
        Self {
            notice: notice.into(),
            ..Self::default()
        }
    }
}

#[derive(Template)]
#[template(path = "asset_form.html")]
pub(super) struct AssetEditorPage {
    pub common: Common,
    pub id: Option<Id>,
    pub form: AssetForm,
    pub kinds: Vec<Choice>,
    pub currency_locked: bool,
    pub intrinsic_valuation: bool,
    pub manual_pricing: bool,
    pub price_form: PriceForm,
    pub price_error: String,
    pub splits: Vec<SplitForm>,
    pub split_form: SplitForm,
    pub split_error: String,
    pub quotes: Vec<PriceView>,
    pub transactions: usize,
    pub chart_url: Option<String>,
    pub error: String,
    pub notice: String,
}

impl AssetEditorPage {
    pub fn new(
        data: &StoreData,
        asset: Option<&Asset>,
        form: AssetForm,
        feedback: Feedback,
    ) -> Self {
        let id = asset.map(|asset| asset.id);
        Self {
            common: editor_common(data, id),
            id,
            kinds: enum_choices(ASSET_KINDS, &form.kind),
            intrinsic_valuation: form.supports_intrinsic_valuation(),
            form,
            currency_locked: asset.is_some_and(|asset| currency_in_use(data, asset)),
            manual_pricing: asset.is_some_and(Asset::allows_manual_pricing),
            price_form: PriceForm {
                currency: data.config.selected_base_currency.clone(),
                ..PriceForm::default()
            },
            price_error: String::new(),
            splits: split_forms(data, id),
            split_form: SplitForm::default(),
            split_error: String::new(),
            quotes: prices(data)
                .into_iter()
                .filter(|price| Some(price.asset_id) == id)
                .collect(),
            transactions: transaction_count(data, id),
            chart_url: id.and_then(|id| chart_url(data, id)),
            error: feedback.error,
            notice: feedback.notice,
        }
    }

    pub fn with_price_error(mut self, form: PriceForm, error: String) -> Self {
        self.price_form = form;
        self.price_error = error;
        self
    }

    pub fn with_split_error(mut self, form: SplitForm, error: String) -> Self {
        self.split_form = form;
        self.split_error = error;
        self
    }
}

fn editor_common(data: &StoreData, id: Option<Id>) -> Common {
    let title = if id.is_some() {
        "Edit asset"
    } else {
        "Add asset"
    };
    let path = id
        .map(|id| format!("/assets/{id}"))
        .unwrap_or_else(|| "/assets/new".into());
    Common::new(data, title, "assets", path)
}

fn transaction_count(data: &StoreData, id: Option<Id>) -> usize {
    id.map(|id| {
        data.transactions
            .iter()
            .filter(|transaction| {
                transaction.base_asset_id == id
                    || transaction.quote_asset_id == Some(id)
                    || transaction.fee_asset_id == Some(id)
            })
            .count()
    })
    .unwrap_or_default()
}
