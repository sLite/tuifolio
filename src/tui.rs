use std::io::{self, stdout};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    accounting::{Report, build_report},
    model::StoreData,
};

#[derive(Clone, Copy)]
enum Screen {
    Home,
    Portfolios,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PortfolioFocus {
    List,
    Holdings,
}

struct AppState {
    screen: Screen,
    selected_portfolio: usize,
    selected_home_holding: usize,
    selected_portfolio_holding: usize,
    portfolio_focus: PortfolioFocus,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            screen: Screen::Home,
            selected_portfolio: 0,
            selected_home_holding: 0,
            selected_portfolio_holding: 0,
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
        clamp_selected_portfolio(&mut state, &report);
        terminal.draw(|frame| render(frame, &report, &state))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => break Ok(()),
                KeyCode::Char('1') => state.screen = Screen::Home,
                KeyCode::Char('2') => state.screen = Screen::Portfolios,
                KeyCode::Char('b') => cycle_base_currency(data),
                KeyCode::Tab if matches!(state.screen, Screen::Portfolios) => {
                    toggle_portfolio_focus(&mut state)
                }
                KeyCode::Up | KeyCode::Char('k') => select_previous_row(&mut state, &report),
                KeyCode::Down | KeyCode::Char('j') => select_next_row(&mut state, &report),
                _ => {}
            }
        }
    }
}

fn render(frame: &mut Frame, report: &Report, state: &AppState) {
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
    }
    frame.render_widget(
        " 1 Home  2 Portfolios  Tab Switch Pane  ↑/↓ or j/k Navigate  b Base Currency  q Quit "
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
    let selected_name = report
        .portfolios
        .get(state.selected_portfolio)
        .map(|portfolio| portfolio.name.as_str())
        .unwrap_or_default();

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
                Cell::from(money(portfolio.value)),
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
        .filter(|holding| holding.portfolio == selected_name)
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

fn clamp_selected_portfolio(state: &mut AppState, report: &Report) {
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
}

fn toggle_portfolio_focus(state: &mut AppState) {
    state.portfolio_focus = match state.portfolio_focus {
        PortfolioFocus::List => PortfolioFocus::Holdings,
        PortfolioFocus::Holdings => PortfolioFocus::List,
    };
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
    }
}

fn select_next_row(state: &mut AppState, report: &Report) {
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
    }
}

fn selected_portfolio_holding_count(state: &AppState, report: &Report) -> usize {
    let selected_name = report
        .portfolios
        .get(state.selected_portfolio)
        .map(|portfolio| portfolio.name.as_str())
        .unwrap_or_default();
    report
        .holdings
        .iter()
        .filter(|holding| holding.portfolio == selected_name)
        .count()
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

fn money(value: rust_decimal::Decimal) -> String {
    fixed_2(value)
}

fn fixed_2(value: rust_decimal::Decimal) -> String {
    format!("{:.2}", value.round_dp(2))
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
    quantity.round_dp(6).to_string()
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
        format!("{} ({}%)", money(pnl), fixed_2(percent))
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
