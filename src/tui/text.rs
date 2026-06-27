use chrono::{DateTime, Local};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::AGENTS;

pub(super) fn char_to_byte_idx(value: &str, char_idx: usize) -> usize {
    value
        .char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or(value.len())
}

pub(super) fn display_width_until(value: &str, char_idx: usize) -> usize {
    value.chars().take(char_idx).collect::<String>().width()
}

pub(super) fn line_width(line: &Line) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

pub(super) fn truncate(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if value.width() <= width {
        return value.to_string();
    }
    let keep = width.saturating_sub(3);
    let mut out = String::new();
    for ch in value.chars() {
        if out.width() + ch.width().unwrap_or(0) > keep {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

pub(super) fn search_query_spans(query: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut token_start = None;

    for (idx, ch) in query.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = token_start.take() {
                push_search_token(&mut spans, &query[start..idx]);
            }
            spans.push(Span::raw(ch.to_string()));
        } else if token_start.is_none() {
            token_start = Some(idx);
        }
    }

    if let Some(start) = token_start {
        push_search_token(&mut spans, &query[start..]);
    }

    spans
}

fn push_search_token(spans: &mut Vec<Span<'static>>, token: &str) {
    let (neg, rest) = token
        .strip_prefix('-')
        .map(|rest| (Some("-"), rest))
        .unwrap_or((None, token));
    let Some(keyword) = ["agent:", "dir:", "date:"]
        .iter()
        .find(|keyword| rest.starts_with(**keyword))
    else {
        spans.push(Span::raw(token.to_string()));
        return;
    };
    let value = &rest[keyword.len()..];
    if value.is_empty() {
        spans.push(Span::raw(token.to_string()));
        return;
    }

    let valid = valid_search_filter_value(keyword, value);
    if let Some(neg) = neg {
        spans.push(Span::styled(
            neg.to_string(),
            Style::new().fg(Color::Red).bold(),
        ));
    }
    spans.push(Span::styled(
        keyword.to_string(),
        if valid {
            Style::new().fg(Color::Cyan).bold()
        } else {
            Style::new().fg(Color::Red).bold()
        },
    ));
    if !valid {
        spans.push(Span::styled(
            value.to_string(),
            Style::new()
                .fg(Color::Red)
                .add_modifier(Modifier::CROSSED_OUT),
        ));
    } else if let Some(value) = value.strip_prefix('!') {
        spans.push(Span::styled(
            "!".to_string(),
            Style::new().fg(Color::Red).bold(),
        ));
        spans.push(Span::styled(
            value.to_string(),
            Style::new().fg(Color::Green),
        ));
    } else {
        spans.push(Span::styled(
            value.to_string(),
            Style::new().fg(Color::Green),
        ));
    }
}

fn valid_search_filter_value(keyword: &str, value: &str) -> bool {
    let value = value.trim_start_matches('!');
    let values: Vec<_> = value
        .split(',')
        .map(|part| part.trim().trim_start_matches('!'))
        .filter(|part| !part.is_empty())
        .collect();

    match keyword {
        "agent:" => values
            .iter()
            .all(|value| AGENTS.contains_key(value.to_ascii_lowercase().as_str())),
        "date:" => values.iter().all(|value| valid_search_date_value(value)),
        "dir:" => true,
        _ => true,
    }
}

fn valid_search_date_value(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    if matches!(value.as_str(), "today" | "yesterday" | "week" | "month") {
        return true;
    }
    let rest = value
        .strip_prefix('<')
        .or_else(|| value.strip_prefix('>'))
        .unwrap_or(&value);
    let digit_count = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_count == 0 {
        return false;
    }
    let unit = &rest[digit_count..];
    matches!(unit, "m" | "h" | "d" | "w" | "mo" | "y")
}

pub(super) fn time_ago(timestamp: DateTime<Local>) -> String {
    let delta = Local::now().signed_duration_since(timestamp);
    if delta.num_minutes() < 1 {
        "now".to_string()
    } else if delta.num_hours() < 1 {
        format!("{}m", delta.num_minutes())
    } else if delta.num_days() < 1 {
        format!("{}h", delta.num_hours())
    } else if delta.num_days() < 30 {
        format!("{}d", delta.num_days())
    } else if delta.num_days() < 365 {
        format!("{}mo", delta.num_days() / 30)
    } else {
        format!("{}y", delta.num_days() / 365)
    }
}

pub(super) fn age_style(timestamp: DateTime<Local>) -> Style {
    let hours = Local::now()
        .signed_duration_since(timestamp)
        .num_hours()
        .max(0) as f64;
    let t = (1.0 - (-0.0149 * hours).exp()).clamp(0.0, 1.0);
    let (r, g, b) = if t < 0.3 {
        let s = t / 0.3;
        (100.0 + s * 100.0, 200.0 - s * 20.0, 50.0 - s * 50.0)
    } else if t < 0.6 {
        let s = (t - 0.3) / 0.3;
        (200.0, 180.0 - s * 80.0, s * 50.0)
    } else {
        let s = (t - 0.6) / 0.4;
        (200.0 - s * 100.0, 100.0, 50.0 + s * 50.0)
    };
    Style::new().fg(Color::Rgb(r as u8, g as u8, b as u8))
}

pub(super) fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_./:=+".contains(ch))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_query_spans_highlight_keywords_like_terminal_ui() {
        let spans = search_query_spans("api -agent:claude date:nope dir:src agent:!codex");
        let rendered = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, "api -agent:claude date:nope dir:src agent:!codex");
        assert_eq!(spans[2].content.as_ref(), "-");
        assert!(spans[2].style.fg == Some(Color::Red));
        assert_eq!(spans[3].content.as_ref(), "agent:");
        assert!(spans[3].style.fg == Some(Color::Cyan));
        assert_eq!(spans[4].content.as_ref(), "claude");
        assert!(spans[4].style.fg == Some(Color::Green));
        assert_eq!(spans[6].content.as_ref(), "date:");
        assert!(spans[6].style.fg == Some(Color::Red));
        assert_eq!(spans[7].content.as_ref(), "nope");
        assert!(spans[7].style.add_modifier.contains(Modifier::CROSSED_OUT));
        assert_eq!(spans[13].content.as_ref(), "!");
        assert!(spans[13].style.fg == Some(Color::Red));
        assert_eq!(spans[14].content.as_ref(), "codex");
        assert!(spans[14].style.fg == Some(Color::Green));
    }
}
