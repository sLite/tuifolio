use crate::{
    model::{Id, Portfolio},
    store::Store,
};

#[derive(Debug, Clone)]
pub struct PortfolioInput {
    pub name: String,
}

pub fn create_portfolio(store: &mut Store, input: PortfolioInput) -> anyhow::Result<Id> {
    let name = input.name.trim();
    anyhow::ensure!(!name.is_empty(), "portfolio name is required");
    anyhow::ensure!(
        name.chars().count() <= 200,
        "portfolio name must be at most 200 characters"
    );
    anyhow::ensure!(
        !store
            .data
            .portfolios
            .iter()
            .any(|portfolio| portfolio.name.trim().eq_ignore_ascii_case(name)),
        "a portfolio with this name already exists"
    );
    let id = store.data.allocate_id();
    store.data.portfolios.push(Portfolio {
        id,
        name: name.into(),
    });
    Ok(id)
}
