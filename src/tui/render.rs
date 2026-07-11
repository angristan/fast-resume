use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::prelude::Frame;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui_image::{Image as TuiImage, protocol::Protocol};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::{AGENT_ORDER, AGENTS, VERSION};
use crate::model::Session;

use super::layout::{self, MainLayout};
use super::preview::render_preview_lines;
use super::state::{AppState, PendingAction, YoloModal};
use super::text::{
    age_style, display_width_until, line_width, search_query_spans, time_ago, truncate,
};

const ACCENT: Color = Color::Rgb(224, 150, 70);
const PANEL_BORDER: Color = Color::Rgb(70, 80, 95);
const SELECTED_BG: Color = Color::Rgb(68, 52, 34);
const WARNING: Color = Color::Rgb(240, 180, 80);
const SEARCH_PLACEHOLDER: &str = "Search titles, messages, paths. Try agent:claude";

pub(super) fn draw(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let layout = layout::app(area, state.show_preview);

    draw_header(frame, layout.header, state);
    draw_search(frame, layout.search, state);
    draw_filters(frame, layout.filters, state);
    draw_main(frame, layout.main, state);
    draw_footer(frame, layout.footer, state);

    if let Some(modal) = &state.modal {
        draw_yolo_modal(frame, area, modal);
    }
}

fn draw_header(frame: &mut Frame, area: Rect, state: &AppState) {
    frame.render_widget(Paragraph::new(header_line(state, area.width)), area);
}

fn header_line(state: &AppState, width: u16) -> Line<'static> {
    let left_spans = vec![
        Span::styled("fast-resume", Style::new().bold().fg(ACCENT)),
        Span::raw(format!(" v{VERSION}")),
    ];
    let count_agent_filter = state.count_agent_filter();
    let count = state.engine.count_for_agent(count_agent_filter.as_deref());
    let right = format!(
        "{} shown / {} indexed   {:.1}ms",
        state.visible.len(),
        count,
        state.last_search_ms
    );
    let right_width = right.width();
    let mut spans = left_spans;
    let refresh_gap = if state.refresh_status.is_empty() {
        0
    } else {
        2
    };
    let base_width = line_width(&Line::from(spans.clone())) + right_width + refresh_gap + 2;
    let refresh_width = (width as usize).saturating_sub(base_width);
    if !state.refresh_status.is_empty() && refresh_width >= 4 {
        let style = if state.scanning {
            Style::new().fg(WARNING)
        } else {
            Style::new().fg(Color::DarkGray)
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            truncate(&state.refresh_status, refresh_width),
            style,
        ));
    }

    let pad = (width as usize)
        .saturating_sub(line_width(&Line::from(spans.clone())))
        .saturating_sub(right_width);
    spans.push(Span::raw(" ".repeat(pad as usize)));
    spans.push(Span::styled(right, Style::new().fg(Color::DarkGray)));
    Line::from(spans)
}

fn draw_search(frame: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(" Search ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt = Span::styled(" / ", Style::new().fg(ACCENT).bold());
    let input_width = inner.width.saturating_sub(3) as usize;
    let mut spans = vec![prompt];
    if state.query.is_empty() {
        if input_width > 0 {
            spans.push(Span::styled(
                truncate(SEARCH_PLACEHOLDER, input_width),
                Style::new().fg(Color::DarkGray).italic(),
            ));
        }
    } else {
        let (visible_query, visible_cursor) =
            search_input_view(&state.query, state.cursor, input_width);
        spans.extend(search_query_spans(&visible_query));
        if visible_cursor == visible_query.chars().count() {
            if let Some(suffix) = state.suggestion_suffix() {
                let remaining = input_width.saturating_sub(visible_query.width());
                if remaining > 0 {
                    spans.push(Span::styled(
                        truncate(&suffix, remaining),
                        Style::new().fg(Color::DarkGray),
                    ));
                }
            }
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);

    let cursor_x =
        inner.x + 3 + search_input_view(&state.query, state.cursor, input_width).1 as u16;
    if cursor_x < inner.right() {
        frame.set_cursor_position((cursor_x, inner.y));
    }
}

fn search_input_view(query: &str, cursor: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }

    let cursor_col = display_width_until(query, cursor);
    let start_col = cursor_col.saturating_sub(width.saturating_sub(1));
    let mut current_col = 0usize;
    let mut start_char = 0usize;
    for (idx, ch) in query.chars().enumerate() {
        let next_col = current_col + ch.width().unwrap_or(0);
        if next_col > start_col {
            start_char = idx;
            break;
        }
        current_col = next_col;
        start_char = idx + 1;
    }

    let actual_start_col = display_width_until(query, start_char);
    let cursor_in_view = cursor_col.saturating_sub(actual_start_col);
    let mut out = String::new();
    for ch in query.chars().skip(start_char) {
        let ch_width = ch.width().unwrap_or(0);
        if out.width() + ch_width > width {
            break;
        }
        out.push(ch);
    }

    (out, cursor_in_view.min(width.saturating_sub(1)))
}

fn draw_filters(frame: &mut Frame, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let active_agents = state.active_agent_filters();
    let mut x = area.x;
    x = draw_filter_tab(
        frame,
        area,
        x,
        "All",
        state.all_agent_filter_active(),
        Color::White,
        None,
    );
    for agent in AGENT_ORDER {
        let config = AGENTS.get(agent).expect("known agent");
        let label = filter_label(config.badge, state.engine.count_for_agent(Some(agent)));
        let active = active_agents.iter().any(|active| active == agent);
        let icon = state
            .images
            .as_ref()
            .and_then(|images| images.row.get(agent));
        x = draw_filter_tab(frame, area, x, &label, active, config.color, icon);
    }
}

fn filter_label(label: &str, count: usize) -> String {
    format!("{label} {count}")
}

fn draw_filter_tab(
    frame: &mut Frame,
    area: Rect,
    x: u16,
    label: &str,
    active: bool,
    color: Color,
    icon: Option<&Protocol>,
) -> u16 {
    let has_icon = icon.is_some();
    let label_width = label.width() as u16;
    let tab_width = label_width + if has_icon { 5 } else { 3 };
    if x >= area.right() {
        return x.saturating_add(tab_width);
    }

    let style = if active {
        Style::new()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(color)
    };

    let visible_width = tab_width.min(area.right().saturating_sub(x));
    frame.render_widget(
        Paragraph::new(" ".repeat(visible_width as usize)).style(style),
        Rect::new(x, area.y, visible_width, 1),
    );

    if let Some(protocol) = icon {
        if x + 1 < area.right() {
            let icon_width = 2.min(area.right().saturating_sub(x + 1));
            frame.render_widget(
                TuiImage::new(protocol).allow_clipping(true),
                Rect::new(x + 1, area.y, icon_width, 1),
            );
        }
        if x + 4 < area.right() {
            frame.render_widget(
                Paragraph::new(truncate(label, area.right().saturating_sub(x + 4) as usize))
                    .style(style),
                Rect::new(x + 4, area.y, label_width.min(area.right() - (x + 4)), 1),
            );
        }
    } else if x + 1 < area.right() {
        frame.render_widget(
            Paragraph::new(truncate(label, area.right().saturating_sub(x + 1) as usize))
                .style(style),
            Rect::new(x + 1, area.y, label_width.min(area.right() - (x + 1)), 1),
        );
    }

    x.saturating_add(tab_width)
}

fn draw_main(frame: &mut Frame, layout: MainLayout, state: &AppState) {
    draw_results(frame, layout.results(), state);
    if let Some(preview) = layout.preview() {
        draw_preview(frame, preview, state);
    }
}

fn draw_results(frame: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(PANEL_BORDER))
        .title(" Sessions ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let columns = result_columns(inner.width);
    draw_result_header(frame, inner, columns);

    let rows_area = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1),
    );

    if state.visible.is_empty() {
        frame.render_widget(
            Paragraph::new("  No sessions found").style(Style::new().fg(Color::DarkGray).italic()),
            rows_area,
        );
        return;
    }

    let max_rows = rows_area.height as usize;
    if max_rows == 0 {
        return;
    }
    let start = state
        .selected
        .saturating_sub(max_rows.saturating_sub(1))
        .min(state.visible.len().saturating_sub(1));
    let end = (start + max_rows).min(state.visible.len());

    for (screen_row, session) in state.visible[start..end].iter().enumerate() {
        let row_y = rows_area.y + screen_row as u16;
        let selected = start + screen_row == state.selected;
        draw_result_row(frame, rows_area, row_y, columns, session, selected, state);
    }
}

#[derive(Clone, Copy)]
struct ResultColumns {
    agent_x: u16,
    agent_w: u16,
    title_x: u16,
    title_w: u16,
    dir_x: u16,
    dir_w: u16,
    turns_x: u16,
    turns_w: u16,
    age_x: u16,
    age_w: u16,
}

fn result_columns(width: u16) -> ResultColumns {
    let (agent_w, dir_w, turns_w, age_w) = if width >= 100 {
        (15, 32, 7, 10)
    } else if width >= 72 {
        (13, 22, 6, 9)
    } else {
        (11, 0, 5, 8)
    };
    let fixed = agent_w + dir_w + turns_w + age_w + 4;
    let title_w = width.saturating_sub(fixed).max(16);
    let agent_x = 0;
    let title_x = agent_x + agent_w + 1;
    let dir_x = title_x + title_w + 1;
    let turns_x = dir_x + dir_w + 1;
    let age_x = turns_x + turns_w + 1;
    ResultColumns {
        agent_x,
        agent_w,
        title_x,
        title_w,
        dir_x,
        dir_w,
        turns_x,
        turns_w,
        age_x,
        age_w,
    }
}

fn draw_result_header(frame: &mut Frame, inner: Rect, columns: ResultColumns) {
    let style = Style::new().fg(Color::Gray).bold();
    draw_cell(
        frame,
        inner,
        columns.agent_x,
        0,
        columns.agent_w,
        "  Agent",
        style,
    );
    draw_cell(
        frame,
        inner,
        columns.title_x,
        0,
        columns.title_w,
        "Title",
        style,
    );
    if columns.dir_w > 0 {
        draw_cell(
            frame,
            inner,
            columns.dir_x,
            0,
            columns.dir_w,
            "Directory",
            style,
        );
    }
    draw_cell(
        frame,
        inner,
        columns.turns_x,
        0,
        columns.turns_w,
        "Turns",
        style,
    );
    draw_cell(frame, inner, columns.age_x, 0, columns.age_w, "Age", style);
}

fn draw_result_row(
    frame: &mut Frame,
    rows_area: Rect,
    row_y: u16,
    columns: ResultColumns,
    session: &Session,
    selected: bool,
    state: &AppState,
) {
    let row_style = if selected {
        Style::new().bg(SELECTED_BG).fg(Color::White)
    } else {
        Style::new()
    };
    frame.render_widget(
        Paragraph::new(" ".repeat(rows_area.width as usize)).style(row_style),
        Rect::new(rows_area.x, row_y, rows_area.width, 1),
    );

    let agent_color = AGENTS
        .get(session.agent.as_str())
        .map(|agent| agent.color)
        .unwrap_or(Color::White);
    let pointer = if selected { "▸" } else { " " };
    draw_cell(
        frame,
        rows_area,
        0,
        row_y - rows_area.y,
        1,
        pointer,
        row_style,
    );

    let label_x = if let Some(protocol) = state
        .images
        .as_ref()
        .and_then(|images| images.row.get(&session.agent))
    {
        frame.render_widget(
            TuiImage::new(protocol).allow_clipping(true),
            Rect::new(rows_area.x + 2, row_y, 2, 1),
        );
        5
    } else {
        2
    };

    draw_cell(
        frame,
        rows_area,
        label_x,
        row_y - rows_area.y,
        columns.agent_w.saturating_sub(label_x),
        &truncate(
            &session.agent,
            columns.agent_w.saturating_sub(label_x) as usize,
        ),
        row_style.fg(agent_color).add_modifier(Modifier::BOLD),
    );
    draw_cell(
        frame,
        rows_area,
        columns.title_x,
        row_y - rows_area.y,
        columns.title_w,
        &truncate(&session.title, columns.title_w as usize),
        row_style,
    );
    if columns.dir_w > 0 {
        draw_cell(
            frame,
            rows_area,
            columns.dir_x,
            row_y - rows_area.y,
            columns.dir_w,
            &truncate(&session.display_directory(), columns.dir_w as usize),
            row_style.fg(Color::DarkGray),
        );
    }
    draw_cell(
        frame,
        rows_area,
        columns.turns_x,
        row_y - rows_area.y,
        columns.turns_w,
        &session.message_count.to_string(),
        row_style,
    );
    draw_cell(
        frame,
        rows_area,
        columns.age_x,
        row_y - rows_area.y,
        columns.age_w,
        &time_ago(session.timestamp),
        age_style(session.timestamp).bg(row_style.bg.unwrap_or(Color::Reset)),
    );
}

fn draw_cell(frame: &mut Frame, area: Rect, x: u16, y: u16, width: u16, text: &str, style: Style) {
    if width == 0 || x >= area.width || y >= area.height {
        return;
    }
    frame.render_widget(
        Paragraph::new(truncate(text, width as usize)).style(style),
        Rect::new(area.x + x, area.y + y, width.min(area.width - x), 1),
    );
}

fn draw_preview(frame: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(PANEL_BORDER))
        .title(" Preview ");
    let inner = block.inner(area).inner(Margin {
        vertical: 0,
        horizontal: 1,
    });
    frame.render_widget(block, area);

    let Some(session) = state.selected_session() else {
        frame.render_widget(Paragraph::new("No session selected").dark_gray(), inner);
        return;
    };

    let agent_color = AGENTS
        .get(session.agent.as_str())
        .map(|agent| agent.color)
        .unwrap_or(Color::White);
    let header_lines = vec![
        Line::from(vec![
            Span::styled(&session.agent, Style::new().fg(agent_color).bold()),
            Span::raw("  "),
            Span::styled(&session.title, Style::new().bold()),
        ]),
        Line::from(vec![
            Span::styled(
                session.display_directory(),
                Style::new().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(
                session.timestamp.format("%Y-%m-%d %H:%M").to_string(),
                Style::new().fg(Color::DarkGray),
            ),
        ]),
    ];

    let mut body_area = inner;
    if let Some(protocol) = state
        .images
        .as_ref()
        .and_then(|images| images.preview.get(&session.agent))
    {
        if inner.width > 48 && inner.height > 7 {
            let logo_area = Rect::new(inner.right().saturating_sub(8), inner.y, 8, 4);
            let text_area = Rect::new(inner.x, inner.y, inner.width.saturating_sub(9), 3);
            frame.render_widget(Paragraph::new(Text::from(header_lines.clone())), text_area);
            frame.render_widget(TuiImage::new(protocol).allow_clipping(true), logo_area);
            body_area = Rect::new(
                inner.x,
                inner.y + 4,
                inner.width,
                inner.height.saturating_sub(4),
            );
        }
    }

    let mut lines = if body_area.y == inner.y {
        let mut lines = header_lines;
        lines.push(Line::raw(""));
        lines
    } else {
        Vec::new()
    };

    lines.extend(
        render_preview_lines(session, &state.query)
            .into_iter()
            .take(220),
    );

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .scroll((state.preview_scroll, 0)),
        body_area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    frame.render_widget(Paragraph::new(footer_line(&state.status, area.width)), area);
}

fn shortcut_footer() -> Line<'static> {
    Line::from(vec![
        Span::styled(" Enter ", Style::new().fg(Color::Black).bg(ACCENT)),
        Span::raw(" resume  "),
        Span::styled(" Ctrl+Y ", Style::new().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" copy  "),
        Span::styled(" Tab ", Style::new().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" agent  "),
        Span::styled(" Ctrl+P ", Style::new().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" preview  "),
        Span::styled(" Esc ", Style::new().fg(Color::Black).bg(Color::Gray)),
        Span::raw(" quit"),
    ])
}

fn footer_line(status: &str, width: u16) -> Line<'static> {
    let shortcuts = shortcut_footer();
    if status.trim().is_empty() {
        return shortcuts;
    }

    let shortcut_width = line_width(&shortcuts);
    let width = width as usize;
    if width <= shortcut_width + 4 {
        return Line::from(Span::styled(
            truncate(status, width),
            Style::new().fg(WARNING),
        ));
    }

    let status_width = width.saturating_sub(shortcut_width + 2);
    let status = truncate(status, status_width);
    let mut spans = vec![
        Span::styled(status, Style::new().fg(WARNING)),
        Span::raw("  "),
    ];
    spans.extend(shortcuts.spans);
    Line::from(spans)
}

fn draw_yolo_modal(frame: &mut Frame, area: Rect, modal: &YoloModal) {
    let popup = centered_rect(48, 8, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(WARNING))
        .title(" Yolo mode ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let text = vec![
        Line::from(yolo_modal_prompt(modal.action)),
        Line::raw(""),
        Line::from(vec![
            button_span(" No ", !modal.selected),
            Span::raw("  "),
            button_span(" Yolo ", modal.selected),
        ])
        .alignment(Alignment::Center),
    ];
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
}

fn yolo_modal_prompt(action: PendingAction) -> &'static str {
    match action {
        PendingAction::Resume => "Resume with auto-approve / skip-permissions flags?",
        PendingAction::Copy => "Copy command with auto-approve / skip-permissions flags?",
    }
}

fn button_span(label: &'static str, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(label, Style::new().fg(Color::Black).bg(WARNING).bold())
    } else {
        Span::styled(label, Style::new().fg(Color::Gray))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_filter_label_includes_session_count() {
        assert_eq!(filter_label("codex", 124), "codex 124");
    }

    #[test]
    fn footer_renders_status_when_present() {
        let line = footer_line("copied: codex resume abc", 120);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("copied: codex resume abc"));
        assert!(rendered.contains("Enter"));
    }

    #[test]
    fn footer_prefers_status_on_narrow_width() {
        let line = footer_line("clipboard unavailable: codex resume abc", 18);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.starts_with("clipboard"));
        assert!(!rendered.contains("Enter"));
    }

    #[test]
    fn yolo_modal_prompt_matches_pending_action() {
        assert_eq!(
            yolo_modal_prompt(PendingAction::Resume),
            "Resume with auto-approve / skip-permissions flags?"
        );
        assert_eq!(
            yolo_modal_prompt(PendingAction::Copy),
            "Copy command with auto-approve / skip-permissions flags?"
        );
    }

    #[test]
    fn search_input_view_keeps_end_cursor_visible() {
        let (visible, cursor) = search_input_view("0123456789abcdef", 16, 6);

        assert_eq!(visible, "bcdef");
        assert_eq!(cursor, 5);
    }

    #[test]
    fn search_input_view_keeps_middle_cursor_visible() {
        let (visible, cursor) = search_input_view("0123456789abcdef", 10, 6);

        assert_eq!(visible, "56789a");
        assert_eq!(cursor, 5);
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width.min(area.width)),
            Constraint::Min(0),
        ])
        .split(area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(horizontal[1]);
    vertical[1]
}
