//! Options browser UI: NixOS and Home-Manager configuration options.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;
use crate::theme::Theme;

/// Draw the options browser (used for both NixOS and HM tabs).
pub fn draw(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    let is_nixos = app.tab == crate::app::Tab::NixosOptions;

    // Extract data references before borrowing app mutably
    let (cursor, scroll) = (app.cursor, app.scroll);
    let results: Vec<OptionRow> = {
        let Some(idx) = app.options_index.as_ref() else {
            let msg = Paragraph::new("Options not loaded").style(Style::default().fg(th.fg));
            f.render_widget(msg, area);
            return;
        };
        if is_nixos {
            app.nixos_results
                .iter()
                .map(|i| {
                    let opt = &idx.doc.nixos_options[*i];
                    OptionRow {
                        name: opt.name.clone(),
                        description: opt.description.clone(),
                        option_type: opt.option_type.clone(),
                        default: opt.default.clone(),
                        example: opt.example.clone(),
                        declared_in: opt.declared_in.clone(),
                    }
                })
                .collect()
        } else {
            app.hm_results
                .iter()
                .map(|i| {
                    let opt = &idx.doc.hm_options[*i];
                    OptionRow {
                        name: opt.name.clone(),
                        description: opt.description.clone(),
                        option_type: opt.option_type.clone(),
                        default: opt.default.clone(),
                        example: opt.example.clone(),
                        declared_in: opt.declared_in.clone(),
                    }
                })
                .collect()
        }
    };

    let width = area.width.max(30);
    let left_w = (width * 55 / 100).min(area.width.saturating_sub(30));
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_w), Constraint::Min(0)])
        .split(area);

    draw_list(f, th, chunks[0], &results, cursor);
    draw_detail(f, th, chunks[1], &results, cursor, scroll);
}

#[derive(Clone)]
struct OptionRow {
    name: String,
    description: String,
    option_type: String,
    default: String,
    example: String,
    declared_in: String,
}

fn draw_list(f: &mut Frame, th: &Theme, area: Rect, results: &[OptionRow], cursor: usize) {
    let header_style = Style::new().fg(th.fg).add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from("Option").style(header_style),
        Cell::from("Type").style(header_style),
    ])
    .height(1);

    let rows: Vec<Row> = results
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let is_selected = i == cursor;
            let style = if is_selected {
                th.selected
            } else {
                Style::default().fg(th.fg)
            };

            Row::new(vec![
                Cell::from(row.name.clone()).style(Style::default().fg(th.accent)),
                Cell::from(row.option_type.clone()).style(Style::default().fg(th.muted)),
            ])
            .style(style)
            .height(1)
        })
        .collect();

    let table = Table::new(
        rows,
        vec![Constraint::Percentage(70), Constraint::Percentage(30)],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(th.border),
    );

    let mut state = ratatui::widgets::TableState::default().with_selected(cursor);
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_detail(
    f: &mut Frame,
    th: &Theme,
    area: Rect,
    results: &[OptionRow],
    cursor: usize,
    scroll: usize,
) {
    let block = Block::default()
        .title("Option Detail")
        .borders(Borders::ALL)
        .border_style(th.border);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(sel) = results.get(cursor) else {
        let msg = Paragraph::new("Select an option").style(Style::default().fg(th.fg));
        f.render_widget(msg, inner);
        return;
    };

    let mut lines = Vec::new();
    let label_style = Style::default().fg(th.muted).add_modifier(Modifier::BOLD);
    let value_style = Style::default().fg(th.fg);

    lines.push(Line::from(vec![
        Span::styled("Name: ", label_style),
        Span::styled(&sel.name, value_style.add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from("&"));
    lines.push(Line::from(vec![
        Span::styled("Description: ", label_style),
        Span::styled(&sel.description, value_style),
    ]));
    lines.push(Line::from("&"));
    lines.push(Line::from(vec![
        Span::styled("Type: ", label_style),
        Span::styled(&sel.option_type, value_style),
    ]));
    if !sel.default.is_empty() {
        lines.push(Line::from("&"));
        lines.push(Line::from(vec![
            Span::styled("Default: ", label_style),
            Span::styled(&sel.default, value_style),
        ]));
    }
    if !sel.example.is_empty() {
        lines.push(Line::from("&"));
        lines.push(Line::from(vec![
            Span::styled("Example: ", label_style),
            Span::styled(&sel.example, value_style),
        ]));
    }
    if !sel.declared_in.is_empty() {
        lines.push(Line::from("&"));
        lines.push(Line::from(vec![
            Span::styled("Declared in: ", label_style),
            Span::styled(&sel.declared_in, Style::default().fg(th.muted)),
        ]));
    }

    let text = Text::from(lines);
    let para = Paragraph::new(text)
        .wrap(ratatui::widgets::Wrap { trim: true })
        .scroll((scroll as u16, 0));
    f.render_widget(para, inner);
}
