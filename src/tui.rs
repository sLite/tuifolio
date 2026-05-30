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
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::{
    accounting::{Report, build_report},
    model::StoreData,
};

#[derive(Clone, Copy)]
enum Screen {
    Home,
    Holdings,
    Portfolios,
}

struct AppState {
    screen: Screen,
    selected_portfolio: usize,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            screen: Screen::Home,
            selected_portfolio: 0,
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
                KeyCode::Char('3') => state.screen = Screen::Holdings,
                KeyCode::Char('b') => cycle_base_currency(data),
                KeyCode::Up | KeyCode::Char('k') if matches!(state.screen, Screen::Portfolios) => {
                    state.selected_portfolio = state.selected_portfolio.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j')
                    if matches!(state.screen, Screen::Portfolios) =>
                {
                    select_next_portfolio(&mut state, &report);
                }
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
        Screen::Home => render_home(frame, report, body),
        Screen::Portfolios => render_portfolios(frame, report, state, body),
        Screen::Holdings => render_holdings(frame, report, body),
    }
    frame.render_widget(
        " 1 Home  2 Portfolios  3 Holdings  ↑/↓ or j/k Select Portfolio  b Base Currency  q Quit "
            .dim(),
        footer,
    );
}

fn header_widget(report: &Report) -> Paragraph<'static> {
    let text = format!(
        "Tuifolio | Base {} | Net worth {} | Unrealized PnL {}",
        report.base_currency,
        money(report.total_value),
        money(report.total_unrealized_pnl)
    );
    Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Dashboard"))
}

fn render_home(frame: &mut Frame, report: &Report, area: ratatui::layout::Rect) {
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
        "Portfolio value: {} {}\nUnrealized PnL: {} {}\nHoldings: {}\nMissing prices: {}\n{}",
        money(report.total_value),
        report.base_currency,
        money(report.total_unrealized_pnl),
        report.base_currency,
        report.holdings.len(),
        unresolved,
        warnings
    );
    frame.render_widget(
        Paragraph::new(text).block(Block::bordered().title("Home")),
        area,
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

    let rows = report
        .portfolios
        .iter()
        .enumerate()
        .map(|(index, portfolio)| {
            let style = if index == state.selected_portfolio {
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            Row::new(vec![
                Cell::from(portfolio.name.clone()),
                Cell::from(money(portfolio.value)),
                Cell::from(portfolio.unresolved.to_string()),
            ])
            .style(style)
        });
    frame.render_widget(portfolio_table(rows), portfolio_area);

    let rows = report
        .holdings
        .iter()
        .filter(|holding| holding.portfolio == selected_name)
        .map(portfolio_holding_row);
    frame.render_widget(
        portfolio_holding_table(rows)
            .block(Block::bordered().title(format!("Assets: {selected_name}"))),
        holdings_area,
    );
}

fn render_holdings(frame: &mut Frame, report: &Report, area: ratatui::layout::Rect) {
    let rows = report.holdings.iter().take(100).map(holding_row);
    frame.render_widget(holding_table(rows), area);
}

fn holding_row(holding: &crate::accounting::HoldingRow) -> Row<'static> {
    let style = if holding.quantity < rust_decimal::Decimal::ZERO {
        Style::new().fg(Color::Red)
    } else {
        Style::new()
    };
    Row::new(vec![
        Cell::from(holding.portfolio.clone()),
        Cell::from(holding.symbol.clone()),
        Cell::from(holding.quantity.round_dp(6).to_string()),
        Cell::from(holding.value.map(money).unwrap_or_else(|| "n/a".into())),
        Cell::from(
            holding
                .unrealized_pnl
                .map(money)
                .unwrap_or_else(|| "n/a".into()),
        ),
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

fn portfolio_holding_row(holding: &crate::accounting::HoldingRow) -> Row<'static> {
    let style = if holding.quantity < rust_decimal::Decimal::ZERO {
        Style::new().fg(Color::Red)
    } else {
        Style::new()
    };
    Row::new(vec![
        Cell::from(holding.symbol.clone()),
        Cell::from(holding.quantity.round_dp(6).to_string()),
        Cell::from(holding.value.map(money).unwrap_or_else(|| "n/a".into())),
        Cell::from(
            holding
                .unrealized_pnl
                .map(money)
                .unwrap_or_else(|| "n/a".into()),
        ),
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
            Constraint::Length(16),
        ],
    )
    .header(
        Row::new(["Portfolio", "Value", "Missing Prices"])
            .style(Style::new().add_modifier(Modifier::BOLD)),
    )
    .block(Block::bordered().title("Data"))
}

fn holding_table<'a, I>(rows: I) -> Table<'a>
where
    I: IntoIterator<Item = Row<'a>>,
{
    Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Length(20),
            Constraint::Length(18),
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new([
            "Portfolio",
            "Asset",
            "Qty",
            "Value",
            "Unrealized PnL",
            "Cost Basis",
            "Name",
        ])
        .style(Style::new().add_modifier(Modifier::BOLD)),
    )
    .block(Block::bordered().title("Data"))
}

fn portfolio_holding_table<'a, I>(rows: I) -> Table<'a>
where
    I: IntoIterator<Item = Row<'a>>,
{
    Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new([
            "Asset",
            "Qty",
            "Value",
            "Unrealized PnL",
            "Cost Basis",
            "Name",
        ])
        .style(Style::new().add_modifier(Modifier::BOLD)),
    )
    .block(Block::bordered().title("Data"))
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
    if report.portfolios.is_empty() {
        state.selected_portfolio = 0;
    } else if state.selected_portfolio >= report.portfolios.len() {
        state.selected_portfolio = report.portfolios.len() - 1;
    }
}

fn select_next_portfolio(state: &mut AppState, report: &Report) {
    if state.selected_portfolio + 1 < report.portfolios.len() {
        state.selected_portfolio += 1;
    }
}

fn money(value: rust_decimal::Decimal) -> String {
    value.round_dp(2).to_string()
}
