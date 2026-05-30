use std::io::{self, stdout};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table},
};

use crate::{
    accounting::{Report, build_report},
    formatting::{money, quantity as format_quantity},
    model::{Asset, Id, Portfolio, StoreData, Transaction},
};

#[derive(Clone, Copy)]
enum Screen {
    Home,
    Portfolios,
    Transactions,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PortfolioFocus {
    List,
    Holdings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransactionFocus {
    Table,
    Portfolio,
    Asset,
    Scope,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AssetScope {
    Primary,
    AnySide,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilterPopup {
    Portfolio,
    Asset,
    Scope,
}

struct AppState {
    screen: Screen,
    selected_portfolio: usize,
    selected_home_holding: usize,
    selected_portfolio_holding: usize,
    selected_transaction: usize,
    transaction_portfolio_filter: Option<Id>,
    transaction_asset_filter: Option<Id>,
    transaction_focus: TransactionFocus,
    transaction_asset_scope: AssetScope,
    filter_popup: Option<FilterPopup>,
    selected_filter_value: usize,
    portfolio_focus: PortfolioFocus,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            screen: Screen::Home,
            selected_portfolio: 0,
            selected_home_holding: 0,
            selected_portfolio_holding: 0,
            selected_transaction: 0,
            transaction_portfolio_filter: None,
            transaction_asset_filter: None,
            transaction_focus: TransactionFocus::Table,
            transaction_asset_scope: AssetScope::Primary,
            filter_popup: None,
            selected_filter_value: 0,
            portfolio_focus: PortfolioFocus::List,
        }
    }
}

pub fn run(data: &mut StoreData) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let result = run_loop(data, &mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_loop(
    data: &mut StoreData,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> io::Result<()> {
    let mut state = AppState::default();
    loop {
        let report = build_report(data);
        clamp_state(&mut state, &report, data);
        terminal.draw(|frame| render(frame, &report, data, &state))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if handle_popup_key(&mut state, data, key.code) {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => break Ok(()),
                KeyCode::Char('1') => state.screen = Screen::Home,
                KeyCode::Char('2') => state.screen = Screen::Portfolios,
                KeyCode::Char('3') => open_transactions_default(&mut state),
                KeyCode::Char('b') => cycle_base_currency(data),
                KeyCode::Tab if matches!(state.screen, Screen::Portfolios) => {
                    toggle_portfolio_focus(&mut state)
                }
                KeyCode::Tab if matches!(state.screen, Screen::Transactions) => {
                    toggle_transaction_pane_focus(&mut state)
                }
                KeyCode::Enter => drill_down(&mut state, &report, data),
                KeyCode::Up | KeyCode::Char('k') => select_previous_row(&mut state, &report),
                KeyCode::Down | KeyCode::Char('j') => select_next_row(&mut state, &report, data),
                _ => {}
            }
        }
    }
}

fn render(frame: &mut Frame, report: &Report, data: &StoreData, state: &AppState) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(header_widget(report), header);
    match state.screen {
        Screen::Home => render_home(frame, report, state, body),
        Screen::Portfolios => render_portfolios(frame, report, state, body),
        Screen::Transactions => render_transactions(frame, data, state, body),
    }
    frame.render_widget(
        " 1 Home  2 Portfolios  3 Transactions  Enter Open/Select  Esc Close Popup  Tab Switch  ↑/↓ Navigate  b Base  q Quit "
            .dim(),
        footer,
    );
}

fn header_widget(report: &Report) -> Paragraph<'static> {
    let text = format!(
        " Base {}  Net worth {}  Unrealized PnL {} ",
        report.base_currency,
        money(report.net_value),
        money(report.total_unrealized_pnl)
    );
    Paragraph::new(text)
        .style(Style::new().fg(Color::White).add_modifier(Modifier::BOLD))
        .block(panel("Tuifolio", true))
}

fn render_home(frame: &mut Frame, report: &Report, state: &AppState, area: ratatui::layout::Rect) {
    let [summary_area, holdings_area] =
        Layout::vertical([Constraint::Length(9), Constraint::Fill(1)]).areas(area);
    let warnings = if report.negative_balances.is_empty() {
        "No negative balances detected".to_string()
    } else {
        format!(
            "{} negative balances detected",
            report.negative_balances.len()
        )
    };
    let unresolved = report.holdings.iter().filter(|h| h.stale_price).count();
    let text = format!(
        "Assets: {} {}\nLiabilities: {} {}\nNet worth: {} {}\nUnrealized PnL: {} {}\nHoldings: {}\nMissing prices: {}\n{}",
        money(report.total_assets),
        report.base_currency,
        negative_money(report.total_liabilities),
        report.base_currency,
        money(report.net_value),
        report.base_currency,
        money(report.total_unrealized_pnl),
        report.base_currency,
        report.holdings.len(),
        unresolved,
        warnings
    );
    frame.render_widget(
        Paragraph::new(text).block(panel("Home", false)),
        summary_area,
    );
    let visible_rows = table_visible_rows(holdings_area);
    let offset = scroll_offset(
        state.selected_home_holding,
        report.holdings.len(),
        visible_rows,
    );
    let rows = report
        .holdings
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows)
        .map(|(index, holding)| holding_row(holding, index == state.selected_home_holding));
    frame.render_widget(
        holding_table(rows).block(panel("Holdings", true)),
        holdings_area,
    );
}

fn render_portfolios(
    frame: &mut Frame,
    report: &Report,
    state: &AppState,
    area: ratatui::layout::Rect,
) {
    let [portfolio_area, holdings_area] =
        Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(area);
    let selected_portfolio = report
        .portfolios
        .get(state.selected_portfolio)
        .map(|portfolio| (portfolio.id, portfolio.name.as_str()));
    let selected_name = selected_portfolio.map(|(_, name)| name).unwrap_or_default();

    let portfolio_visible_rows = table_visible_rows(portfolio_area);
    let portfolio_offset = scroll_offset(
        state.selected_portfolio,
        report.portfolios.len(),
        portfolio_visible_rows,
    );
    let rows = report
        .portfolios
        .iter()
        .enumerate()
        .skip(portfolio_offset)
        .take(portfolio_visible_rows)
        .map(|(index, portfolio)| {
            let style = if index == state.selected_portfolio {
                selected_row_style(state.portfolio_focus == PortfolioFocus::List)
            } else {
                Style::new()
            };
            Row::new(vec![
                Cell::from(portfolio.name.clone()),
                Cell::from(money(portfolio.assets)),
                Cell::from(negative_money(portfolio.liabilities)),
                Cell::from(money(portfolio.net_value)),
                Cell::from(portfolio.unresolved.to_string()),
            ])
            .style(style)
        });
    frame.render_widget(
        portfolio_table(rows).block(panel(
            "Portfolios",
            state.portfolio_focus == PortfolioFocus::List,
        )),
        portfolio_area,
    );

    let selected_holdings = report
        .holdings
        .iter()
        .filter(|holding| {
            selected_portfolio
                .map(|(id, _)| holding.portfolio_id == id)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let holding_visible_rows = table_visible_rows(holdings_area);
    let holding_offset = scroll_offset(
        state.selected_portfolio_holding,
        selected_holdings.len(),
        holding_visible_rows,
    );
    let rows = selected_holdings
        .iter()
        .enumerate()
        .skip(holding_offset)
        .take(holding_visible_rows)
        .map(|(index, holding)| {
            portfolio_holding_row(
                holding,
                state.portfolio_focus == PortfolioFocus::Holdings
                    && index == state.selected_portfolio_holding,
            )
        });
    frame.render_widget(
        portfolio_holding_table(rows).block(panel(
            format!("Assets: {selected_name}"),
            state.portfolio_focus == PortfolioFocus::Holdings,
        )),
        holdings_area,
    );
}

fn render_transactions(
    frame: &mut Frame,
    data: &StoreData,
    state: &AppState,
    area: ratatui::layout::Rect,
) {
    let [filter_area, table_area] =
        Layout::vertical([Constraint::Length(6), Constraint::Fill(1)]).areas(area);
    frame.render_widget(transaction_filter_widget(data, state), filter_area);

    let transactions = filtered_transactions(data, state);
    let visible_rows = table_visible_rows(table_area);
    let offset = scroll_offset(state.selected_transaction, transactions.len(), visible_rows);
    let rows = transactions
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows)
        .map(|(index, transaction)| {
            transaction_row(data, transaction, index == state.selected_transaction)
        });
    frame.render_widget(
        transaction_table(rows).block(panel(
            format!("Transactions ({})", transactions.len()),
            state.transaction_focus == TransactionFocus::Table,
        )),
        table_area,
    );
    if let Some(popup) = state.filter_popup {
        render_filter_popup(frame, data, state, popup, area);
    }
}

fn transaction_filter_widget(data: &StoreData, state: &AppState) -> Paragraph<'static> {
    let portfolio = selected_filter_portfolio(data, state)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "All".into());
    let asset = selected_filter_asset(data, state)
        .map(|a| a.symbol.clone())
        .unwrap_or_else(|| "All".into());
    let scope = match state.transaction_asset_scope {
        AssetScope::Primary => "Primary",
        AssetScope::AnySide => "Any side",
    };
    let lines = vec![
        filter_line(
            "Portfolio",
            portfolio,
            state.transaction_focus == TransactionFocus::Portfolio,
        ),
        filter_line(
            "Asset",
            asset,
            state.transaction_focus == TransactionFocus::Asset,
        ),
        filter_line(
            "Scope",
            scope.to_string(),
            state.transaction_focus == TransactionFocus::Scope,
        ),
        Line::from("Up/Down changes filter focus, Enter opens filter values".dim()),
    ];
    Paragraph::new(lines).block(panel(
        "Filters",
        state.transaction_focus != TransactionFocus::Table,
    ))
}

fn render_filter_popup(
    frame: &mut Frame,
    data: &StoreData,
    state: &AppState,
    popup: FilterPopup,
    area: Rect,
) {
    let popup_area = centered_rect(area, 70, 70);
    let options = filter_options(data, popup);
    let visible_rows = table_visible_rows(popup_area);
    let offset = scroll_offset(state.selected_filter_value, options.len(), visible_rows);
    let rows = options
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows)
        .map(|(index, option)| {
            let style = if index == state.selected_filter_value {
                selected_row_style(true)
            } else {
                Style::new()
            };
            Row::new(vec![Cell::from(option.label.clone())]).style(style)
        });
    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Table::new(rows, [Constraint::Percentage(100)])
            .header(table_header(["Value"]))
            .block(panel(filter_popup_title(popup), true)),
        popup_area,
    );
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let [area] = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(ratatui::layout::Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(ratatui::layout::Flex::Center)
        .areas(area);
    area
}

fn filter_popup_title(popup: FilterPopup) -> &'static str {
    match popup {
        FilterPopup::Portfolio => "Select Portfolio",
        FilterPopup::Asset => "Select Asset",
        FilterPopup::Scope => "Select Asset Scope",
    }
}

struct FilterOption {
    label: String,
}

fn filter_options(data: &StoreData, popup: FilterPopup) -> Vec<FilterOption> {
    match popup {
        FilterPopup::Portfolio => std::iter::once(FilterOption {
            label: "All".into(),
        })
        .chain(data.portfolios.iter().map(|portfolio| FilterOption {
            label: portfolio.name.clone(),
        }))
        .collect(),
        FilterPopup::Asset => std::iter::once(FilterOption {
            label: "All".into(),
        })
        .chain(data.assets.iter().map(|asset| FilterOption {
            label: format!("{} - {}", asset.symbol, asset.name),
        }))
        .collect(),
        FilterPopup::Scope => vec![
            FilterOption {
                label: "Primary/base asset only".into(),
            },
            FilterOption {
                label: "Any side: base, quote, or fee".into(),
            },
        ],
    }
}

fn filter_line(label: &'static str, value: String, active: bool) -> Line<'static> {
    let value_style = if active {
        Style::new()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::White)
    };
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::new().fg(Color::Gray)),
        Span::styled(format!(" {value} "), value_style),
    ])
}

fn filtered_transactions<'a>(data: &'a StoreData, state: &AppState) -> Vec<&'a Transaction> {
    let portfolio_id = selected_filter_portfolio(data, state).map(|portfolio| portfolio.id);
    let asset_id = selected_filter_asset(data, state).map(|asset| asset.id);
    let mut transactions = data
        .transactions
        .iter()
        .filter(|transaction| portfolio_id.is_none_or(|id| transaction.portfolio_id == id))
        .filter(|transaction| asset_matches(transaction, asset_id, state.transaction_asset_scope))
        .collect::<Vec<_>>();
    transactions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| b.id.cmp(&a.id)));
    transactions
}

fn asset_matches(transaction: &Transaction, asset_id: Option<Id>, scope: AssetScope) -> bool {
    let Some(asset_id) = asset_id else {
        return true;
    };
    match scope {
        AssetScope::Primary => transaction.base_asset_id == asset_id,
        AssetScope::AnySide => {
            transaction.base_asset_id == asset_id
                || transaction.quote_asset_id == Some(asset_id)
                || transaction.fee_asset_id == Some(asset_id)
        }
    }
}

fn transaction_row(data: &StoreData, transaction: &Transaction, selected: bool) -> Row<'static> {
    let style = if selected {
        selected_row_style(true)
    } else {
        Style::new()
    };
    Row::new(vec![
        Cell::from(transaction.timestamp.format("%Y-%m-%d").to_string()),
        Cell::from(portfolio_name(data, transaction.portfolio_id)),
        Cell::from(format!("{:?}", transaction.kind)),
        Cell::from(asset_amount(
            data,
            transaction.base_asset_id,
            Some(transaction.base_amount),
        )),
        Cell::from(optional_asset_amount(
            data,
            transaction.quote_asset_id,
            transaction.quote_amount,
        )),
        Cell::from(optional_asset_amount(
            data,
            transaction.fee_asset_id,
            transaction.fee_amount,
        )),
        Cell::from(transaction.exchange.clone().unwrap_or_default()),
        Cell::from(transaction.notes.clone().unwrap_or_default()),
    ])
    .style(style)
}

fn holding_row(holding: &crate::accounting::HoldingRow, selected: bool) -> Row<'static> {
    let style = if selected {
        selected_row_style(true)
    } else if matches!(holding.kind, crate::model::AssetKind::Liability) {
        Style::new().fg(Color::Gray)
    } else {
        Style::new()
    };
    Row::new(vec![
        Cell::from(holding.portfolio.clone()),
        Cell::from(holding.symbol.clone()),
        numeric_cell(
            display_quantity(holding),
            selected,
            display_quantity_is_negative(holding),
        ),
        numeric_cell(
            display_value(holding),
            selected,
            display_value_is_negative(holding),
        ),
        pnl_cell(holding, selected),
        Cell::from(
            holding
                .net_invested
                .map(money)
                .unwrap_or_else(|| "n/a".into()),
        ),
        Cell::from(holding.name.clone()),
    ])
    .style(style)
}

fn portfolio_holding_row(holding: &crate::accounting::HoldingRow, selected: bool) -> Row<'static> {
    let style = if selected {
        selected_row_style(true)
    } else if matches!(holding.kind, crate::model::AssetKind::Liability) {
        Style::new().fg(Color::Gray)
    } else {
        Style::new()
    };
    Row::new(vec![
        Cell::from(holding.symbol.clone()),
        numeric_cell(
            display_quantity(holding),
            selected,
            display_quantity_is_negative(holding),
        ),
        numeric_cell(
            display_value(holding),
            selected,
            display_value_is_negative(holding),
        ),
        pnl_cell(holding, selected),
        Cell::from(
            holding
                .net_invested
                .map(money)
                .unwrap_or_else(|| "n/a".into()),
        ),
        Cell::from(holding.name.clone()),
    ])
    .style(style)
}

fn portfolio_table<'a, I>(rows: I) -> Table<'a>
where
    I: IntoIterator<Item = Row<'a>>,
{
    Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Length(16),
        ],
    )
    .header(table_header([
        "Portfolio",
        "Assets",
        "Liabilities",
        "Net",
        "Missing Prices",
    ]))
}

fn holding_table<'a, I>(rows: I) -> Table<'a>
where
    I: IntoIterator<Item = Row<'a>>,
{
    Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(18),
            Constraint::Length(16),
            Constraint::Length(30),
            Constraint::Length(16),
            Constraint::Min(0),
        ],
    )
    .header(table_header([
        "Portfolio",
        "Asset",
        "Qty",
        "Value",
        "Unrealized PnL",
        "Cost Basis",
        "Name",
    ]))
}

fn portfolio_holding_table<'a, I>(rows: I) -> Table<'a>
where
    I: IntoIterator<Item = Row<'a>>,
{
    Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Length(18),
            Constraint::Length(16),
            Constraint::Length(30),
            Constraint::Length(16),
            Constraint::Min(0),
        ],
    )
    .header(table_header([
        "Asset",
        "Qty",
        "Value",
        "Unrealized PnL",
        "Cost Basis",
        "Name",
    ]))
}

fn transaction_table<'a, I>(rows: I) -> Table<'a>
where
    I: IntoIterator<Item = Row<'a>>,
{
    Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(18),
            Constraint::Length(20),
            Constraint::Length(20),
            Constraint::Length(16),
            Constraint::Length(12),
            Constraint::Min(0),
        ],
    )
    .header(table_header([
        "Date",
        "Portfolio",
        "Kind",
        "Base",
        "Quote",
        "Fee",
        "Exchange",
        "Notes",
    ]))
}

fn portfolio_name(data: &StoreData, id: Id) -> String {
    data.portfolios
        .iter()
        .find(|portfolio| portfolio.id == id)
        .map(|portfolio| portfolio.name.clone())
        .unwrap_or_default()
}

fn asset_amount(data: &StoreData, asset_id: Id, amount: Option<rust_decimal::Decimal>) -> String {
    let symbol = data
        .assets
        .iter()
        .find(|asset| asset.id == asset_id)
        .map(|asset| asset.symbol.as_str())
        .unwrap_or("?");
    amount
        .map(|amount| format!("{} {symbol}", format_quantity(amount)))
        .unwrap_or_default()
}

fn optional_asset_amount(
    data: &StoreData,
    asset_id: Option<Id>,
    amount: Option<rust_decimal::Decimal>,
) -> String {
    asset_id
        .map(|asset_id| asset_amount(data, asset_id, amount))
        .unwrap_or_default()
}

fn panel<T: Into<String>>(title: T, active: bool) -> Block<'static> {
    let title = title.into();
    let color = if active { Color::Cyan } else { Color::DarkGray };
    Block::default()
        .borders(Borders::ALL)
        .border_type(if active {
            BorderType::Rounded
        } else {
            BorderType::Plain
        })
        .border_style(Style::new().fg(color))
        .title(Line::from(vec![Span::styled(
            format!(" {title} "),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        )]))
}

fn table_header<const N: usize>(columns: [&'static str; N]) -> Row<'static> {
    Row::new(columns).style(Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD))
}

fn selected_row_style(active: bool) -> Style {
    if active {
        Style::new()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD | Modifier::DIM)
    }
}

fn cycle_base_currency(data: &mut StoreData) {
    let currencies = &data.config.base_currencies;
    if currencies.is_empty() {
        return;
    }
    let current = currencies
        .iter()
        .position(|c| c == &data.config.selected_base_currency)
        .unwrap_or(0);
    data.config.selected_base_currency = currencies[(current + 1) % currencies.len()].clone();
}

fn handle_popup_key(state: &mut AppState, data: &StoreData, key: KeyCode) -> bool {
    let Some(popup) = state.filter_popup else {
        return false;
    };
    match key {
        KeyCode::Esc => state.filter_popup = None,
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected_filter_value = state.selected_filter_value.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let len = filter_options(data, popup).len();
            select_next_index(&mut state.selected_filter_value, len);
        }
        KeyCode::Enter => select_filter_popup_value(state, data, popup),
        _ => {}
    }
    true
}

fn select_filter_popup_value(state: &mut AppState, data: &StoreData, popup: FilterPopup) {
    match popup {
        FilterPopup::Portfolio => {
            state.transaction_portfolio_filter = if state.selected_filter_value == 0 {
                None
            } else {
                data.portfolios
                    .get(state.selected_filter_value - 1)
                    .map(|portfolio| portfolio.id)
            };
        }
        FilterPopup::Asset => {
            state.transaction_asset_filter = if state.selected_filter_value == 0 {
                None
            } else {
                data.assets
                    .get(state.selected_filter_value - 1)
                    .map(|asset| asset.id)
            };
        }
        FilterPopup::Scope => {
            state.transaction_asset_scope = if state.selected_filter_value == 0 {
                AssetScope::Primary
            } else {
                AssetScope::AnySide
            };
        }
    }
    state.selected_transaction = 0;
    state.filter_popup = None;
}

fn clamp_state(state: &mut AppState, report: &Report, data: &StoreData) {
    state.selected_home_holding = clamp_index(state.selected_home_holding, report.holdings.len());
    if report.portfolios.is_empty() {
        state.selected_portfolio = 0;
    } else if state.selected_portfolio >= report.portfolios.len() {
        state.selected_portfolio = report.portfolios.len() - 1;
    }
    state.selected_portfolio_holding = clamp_index(
        state.selected_portfolio_holding,
        selected_portfolio_holding_count(state, report),
    );
    if !data
        .portfolios
        .iter()
        .any(|portfolio| Some(portfolio.id) == state.transaction_portfolio_filter)
    {
        state.transaction_portfolio_filter = None;
    }
    if !data
        .assets
        .iter()
        .any(|asset| Some(asset.id) == state.transaction_asset_filter)
    {
        state.transaction_asset_filter = None;
    }
    state.selected_transaction = clamp_index(
        state.selected_transaction,
        filtered_transactions(data, state).len(),
    );
    if let Some(popup) = state.filter_popup {
        state.selected_filter_value = clamp_index(
            state.selected_filter_value,
            filter_options(data, popup).len(),
        );
    }
}

fn toggle_portfolio_focus(state: &mut AppState) {
    state.portfolio_focus = match state.portfolio_focus {
        PortfolioFocus::List => PortfolioFocus::Holdings,
        PortfolioFocus::Holdings => PortfolioFocus::List,
    };
}

fn toggle_transaction_pane_focus(state: &mut AppState) {
    state.transaction_focus = if state.transaction_focus == TransactionFocus::Table {
        TransactionFocus::Portfolio
    } else {
        TransactionFocus::Table
    };
}

fn next_transaction_filter_focus(state: &mut AppState) {
    state.transaction_focus = match state.transaction_focus {
        TransactionFocus::Table => TransactionFocus::Portfolio,
        TransactionFocus::Portfolio => TransactionFocus::Asset,
        TransactionFocus::Asset => TransactionFocus::Scope,
        TransactionFocus::Scope => TransactionFocus::Portfolio,
    };
}

fn previous_transaction_filter_focus(state: &mut AppState) {
    state.transaction_focus = match state.transaction_focus {
        TransactionFocus::Table => TransactionFocus::Portfolio,
        TransactionFocus::Portfolio => TransactionFocus::Scope,
        TransactionFocus::Asset => TransactionFocus::Portfolio,
        TransactionFocus::Scope => TransactionFocus::Asset,
    };
}

fn drill_down(state: &mut AppState, report: &Report, data: &StoreData) {
    match state.screen {
        Screen::Home => {
            let Some(holding) = report.holdings.get(state.selected_home_holding) else {
                return;
            };
            open_transactions_for_holding(state, holding.portfolio_id, holding.asset_id);
        }
        Screen::Portfolios if state.portfolio_focus == PortfolioFocus::List => {
            let Some(portfolio) = report.portfolios.get(state.selected_portfolio) else {
                return;
            };
            state.screen = Screen::Transactions;
            state.transaction_portfolio_filter = Some(portfolio.id);
            state.transaction_asset_filter = None;
            state.selected_transaction = 0;
        }
        Screen::Portfolios => {
            let holdings = selected_portfolio_holdings(state, report);
            let Some(holding) = holdings.get(state.selected_portfolio_holding) else {
                return;
            };
            open_transactions_for_holding(state, holding.portfolio_id, holding.asset_id);
        }
        Screen::Transactions => open_filter_popup(state, data),
    }
}

fn open_filter_popup(state: &mut AppState, data: &StoreData) {
    let Some(popup) = focused_filter_popup(state.transaction_focus) else {
        return;
    };
    state.selected_filter_value = selected_filter_value_index(data, state, popup);
    state.filter_popup = Some(popup);
}

fn focused_filter_popup(focus: TransactionFocus) -> Option<FilterPopup> {
    match focus {
        TransactionFocus::Portfolio => Some(FilterPopup::Portfolio),
        TransactionFocus::Asset => Some(FilterPopup::Asset),
        TransactionFocus::Scope => Some(FilterPopup::Scope),
        TransactionFocus::Table => None,
    }
}

fn selected_filter_value_index(data: &StoreData, state: &AppState, popup: FilterPopup) -> usize {
    match popup {
        FilterPopup::Portfolio => state.transaction_portfolio_filter.map_or(0, |id| {
            data.portfolios
                .iter()
                .position(|portfolio| portfolio.id == id)
                .map(|index| index + 1)
                .unwrap_or(0)
        }),
        FilterPopup::Asset => state.transaction_asset_filter.map_or(0, |id| {
            data.assets
                .iter()
                .position(|asset| asset.id == id)
                .map(|index| index + 1)
                .unwrap_or(0)
        }),
        FilterPopup::Scope => match state.transaction_asset_scope {
            AssetScope::Primary => 0,
            AssetScope::AnySide => 1,
        },
    }
}

fn open_transactions_for_holding(state: &mut AppState, portfolio_id: Id, asset_id: Id) {
    state.screen = Screen::Transactions;
    state.transaction_portfolio_filter = Some(portfolio_id);
    state.transaction_asset_filter = Some(asset_id);
    state.transaction_asset_scope = AssetScope::Primary;
    state.transaction_focus = TransactionFocus::Table;
    state.selected_transaction = 0;
}

fn open_transactions_default(state: &mut AppState) {
    state.screen = Screen::Transactions;
    state.transaction_portfolio_filter = None;
    state.transaction_asset_filter = None;
    state.transaction_asset_scope = AssetScope::Primary;
    state.transaction_focus = TransactionFocus::Table;
    state.selected_transaction = 0;
}

fn select_previous_row(state: &mut AppState, _report: &Report) {
    match state.screen {
        Screen::Home => {
            state.selected_home_holding = state.selected_home_holding.saturating_sub(1);
        }
        Screen::Portfolios if state.portfolio_focus == PortfolioFocus::List => {
            state.selected_portfolio = state.selected_portfolio.saturating_sub(1);
            state.selected_portfolio_holding = 0;
        }
        Screen::Portfolios => {
            state.selected_portfolio_holding = state.selected_portfolio_holding.saturating_sub(1);
        }
        Screen::Transactions if state.transaction_focus == TransactionFocus::Table => {
            state.selected_transaction = state.selected_transaction.saturating_sub(1);
        }
        Screen::Transactions => previous_transaction_filter_focus(state),
    }
}

fn select_next_row(state: &mut AppState, report: &Report, data: &StoreData) {
    match state.screen {
        Screen::Home => select_next_index(&mut state.selected_home_holding, report.holdings.len()),
        Screen::Portfolios if state.portfolio_focus == PortfolioFocus::List => {
            let previous = state.selected_portfolio;
            select_next_index(&mut state.selected_portfolio, report.portfolios.len());
            if state.selected_portfolio != previous {
                state.selected_portfolio_holding = 0;
            }
        }
        Screen::Portfolios => {
            let count = selected_portfolio_holding_count(state, report);
            select_next_index(&mut state.selected_portfolio_holding, count);
        }
        Screen::Transactions if state.transaction_focus == TransactionFocus::Table => {
            let count = filtered_transactions(data, state).len();
            select_next_index(&mut state.selected_transaction, count)
        }
        Screen::Transactions => next_transaction_filter_focus(state),
    }
}

fn selected_portfolio_holding_count(state: &AppState, report: &Report) -> usize {
    let selected_id = report
        .portfolios
        .get(state.selected_portfolio)
        .map(|portfolio| portfolio.id);
    report
        .holdings
        .iter()
        .filter(|holding| selected_id == Some(holding.portfolio_id))
        .count()
}

fn selected_portfolio_holdings<'a>(
    state: &AppState,
    report: &'a Report,
) -> Vec<&'a crate::accounting::HoldingRow> {
    let selected_id = report
        .portfolios
        .get(state.selected_portfolio)
        .map(|portfolio| portfolio.id);
    report
        .holdings
        .iter()
        .filter(|holding| selected_id == Some(holding.portfolio_id))
        .collect()
}

fn selected_filter_portfolio<'a>(data: &'a StoreData, state: &AppState) -> Option<&'a Portfolio> {
    data.portfolios
        .iter()
        .find(|portfolio| Some(portfolio.id) == state.transaction_portfolio_filter)
}

fn selected_filter_asset<'a>(data: &'a StoreData, state: &AppState) -> Option<&'a Asset> {
    data.assets
        .iter()
        .find(|asset| Some(asset.id) == state.transaction_asset_filter)
}

fn select_next_index(selected: &mut usize, len: usize) {
    if *selected + 1 < len {
        *selected += 1;
    }
}

fn clamp_index(selected: usize, len: usize) -> usize {
    if len == 0 { 0 } else { selected.min(len - 1) }
}

fn table_visible_rows(area: ratatui::layout::Rect) -> usize {
    usize::from(area.height.saturating_sub(3))
}

fn scroll_offset(selected: usize, len: usize, visible_rows: usize) -> usize {
    if len <= visible_rows || visible_rows == 0 {
        return 0;
    }
    selected
        .saturating_sub(visible_rows - 1)
        .min(len - visible_rows)
}

fn negative_money(value: rust_decimal::Decimal) -> String {
    if value.is_zero() {
        return money(value);
    }
    money(-value)
}

fn display_quantity(holding: &crate::accounting::HoldingRow) -> String {
    let quantity = if matches!(holding.kind, crate::model::AssetKind::Liability) {
        -holding.quantity
    } else {
        holding.quantity
    };
    format_quantity(quantity)
}

fn display_value(holding: &crate::accounting::HoldingRow) -> String {
    let Some(value) = holding.value else {
        return "n/a".into();
    };
    if matches!(holding.kind, crate::model::AssetKind::Liability) {
        negative_money(value)
    } else {
        money(value)
    }
}

fn numeric_cell(value: String, selected: bool, negative: bool) -> Cell<'static> {
    if selected || !negative {
        Cell::from(value)
    } else {
        Cell::from(value).style(Style::new().fg(Color::LightRed))
    }
}

fn pnl_cell(holding: &crate::accounting::HoldingRow, selected: bool) -> Cell<'static> {
    let Some(pnl) = holding.unrealized_pnl else {
        return Cell::from("n/a");
    };
    let text = if let Some(percent) = unrealized_pnl_percent(holding) {
        format!("{} ({}%)", money(pnl), money(percent))
    } else {
        money(pnl)
    };
    if selected {
        Cell::from(text)
    } else if pnl < rust_decimal::Decimal::ZERO {
        Cell::from(text).style(Style::new().fg(Color::LightRed))
    } else {
        Cell::from(text).style(Style::new().fg(Color::Green))
    }
}

fn unrealized_pnl_percent(
    holding: &crate::accounting::HoldingRow,
) -> Option<rust_decimal::Decimal> {
    let invested = holding.net_invested?;
    if invested.is_zero() {
        return None;
    }
    holding
        .unrealized_pnl
        .map(|pnl| pnl / invested.abs() * rust_decimal::Decimal::new(100, 0))
}

fn display_quantity_is_negative(holding: &crate::accounting::HoldingRow) -> bool {
    if matches!(holding.kind, crate::model::AssetKind::Liability) {
        !holding.quantity.is_zero()
    } else {
        holding.quantity < rust_decimal::Decimal::ZERO
    }
}

fn display_value_is_negative(holding: &crate::accounting::HoldingRow) -> bool {
    matches!(holding.kind, crate::model::AssetKind::Liability)
        && holding.value.map(|value| !value.is_zero()).unwrap_or(false)
}
