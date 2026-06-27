use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::config::AGENTS;
use crate::model::Session;

pub(super) fn render_preview_lines(session: &Session, query: &str) -> Vec<Line<'static>> {
    let snippet = preview_snippet(session, query);
    let agent = AGENTS.get(session.agent.as_str());
    let agent_label = agent.map(|agent| agent.badge).unwrap_or(&session.agent);
    let agent_color = agent.map(|agent| agent.color).unwrap_or(Color::White);
    let terms = preview_terms(query);

    let mut lines = Vec::new();
    for message in snippet.split("\n\n") {
        if message.trim().is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        render_preview_message(&mut lines, message, agent_label, agent_color, &terms);
    }
    lines
}

fn preview_snippet(session: &Session, query: &str) -> String {
    if query.trim().is_empty() {
        return truncate_chars(&session.content, 6_000);
    }

    let terms = preview_terms(query);
    if terms.is_empty() {
        return truncate_chars(&session.content, 6_000);
    }

    let content_lc = session.content.to_ascii_lowercase();
    let mut best = None;
    for term in &terms {
        if let Some(pos) = content_lc.find(term.as_str()) {
            best = Some(best.map_or(pos, |current: usize| current.min(pos)));
        }
    }

    if let Some(pos) = best {
        let start = session.content[..pos]
            .char_indices()
            .rev()
            .nth(220)
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        let end = session.content[pos..]
            .char_indices()
            .nth(5_000)
            .map(|(idx, _)| pos + idx)
            .unwrap_or(session.content.len());
        let mut snippet = String::new();
        if start > 0 {
            snippet.push_str("...\n");
        }
        snippet.push_str(&session.content[start..end]);
        if end < session.content.len() {
            snippet.push_str("\n...");
        }
        snippet
    } else {
        truncate_chars(&session.content, 6_000)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewRole {
    User,
    Assistant,
    Other,
}

fn render_preview_message(
    out: &mut Vec<Line<'static>>,
    message: &str,
    agent_label: &str,
    agent_color: Color,
    terms: &[String],
) {
    let role = preview_role(message);
    let mut in_code = false;
    let mut code_language = String::new();
    let mut first_content_line = true;

    for raw_line in message.lines() {
        let line = strip_role_prefix(raw_line, role);
        if let Some(language) = code_fence_language(line) {
            out.push(render_code_fence_line(
                language,
                role,
                agent_label,
                agent_color,
                first_content_line,
            ));
            if in_code {
                in_code = false;
                code_language.clear();
            } else {
                in_code = true;
                code_language = language.to_ascii_lowercase();
            }
            first_content_line = false;
            continue;
        }

        if in_code {
            out.push(render_code_line(line, &code_language, terms));
            continue;
        }

        match role {
            PreviewRole::User => out.push(render_user_line(line, terms, first_content_line)),
            PreviewRole::Assistant => out.push(render_agent_line(
                line,
                terms,
                agent_label,
                agent_color,
                first_content_line,
            )),
            PreviewRole::Other => out.push(render_plain_preview_line(line, terms)),
        }
        first_content_line = false;
    }
}

fn preview_role(message: &str) -> PreviewRole {
    let trimmed = message.trim_start();
    if trimmed.starts_with("» ") {
        PreviewRole::User
    } else if message.starts_with("  ") {
        PreviewRole::Assistant
    } else {
        PreviewRole::Other
    }
}

fn strip_role_prefix(line: &str, role: PreviewRole) -> &str {
    match role {
        PreviewRole::User => line.strip_prefix("» ").unwrap_or(line),
        PreviewRole::Assistant => line.strip_prefix("  ").unwrap_or(line),
        PreviewRole::Other => line,
    }
}

fn code_fence_language(line: &str) -> Option<&str> {
    line.trim_start()
        .strip_prefix("```")
        .map(str::trim)
        .map(|language| language.split_whitespace().next().unwrap_or_default())
}

fn render_user_line(line: &str, terms: &[String], first: bool) -> Line<'static> {
    let mut spans = Vec::new();
    if first {
        spans.push(Span::styled(
            "» ".to_string(),
            Style::new().fg(Color::Rgb(120, 210, 255)).bold(),
        ));
    } else {
        spans.push(Span::styled(
            "  ".to_string(),
            Style::new().fg(Color::DarkGray),
        ));
    }
    spans.extend(highlight_spans(
        vec![Span::styled(
            line.to_string(),
            Style::new().fg(Color::Rgb(180, 225, 245)).bold(),
        )],
        terms,
    ));
    Line::from(spans)
}

fn render_agent_line(
    line: &str,
    terms: &[String],
    agent_label: &str,
    agent_color: Color,
    first: bool,
) -> Line<'static> {
    let mut spans = Vec::new();
    if first {
        spans.push(Span::styled(
            format!("{agent_label} "),
            Style::new().fg(agent_color).bold(),
        ));
    } else {
        spans.push(Span::styled(
            "  ".to_string(),
            Style::new().fg(Color::DarkGray),
        ));
    }
    spans.extend(highlight_spans(
        vec![Span::styled(
            line.to_string(),
            Style::new().fg(Color::Rgb(220, 225, 230)),
        )],
        terms,
    ));
    Line::from(spans)
}

fn render_plain_preview_line(line: &str, terms: &[String]) -> Line<'static> {
    let style = if line.starts_with("...") {
        Style::new().fg(Color::DarkGray).italic()
    } else {
        Style::new().fg(Color::Gray)
    };
    Line::from(highlight_spans(
        vec![Span::styled(line.to_string(), style)],
        terms,
    ))
}

fn render_code_fence_line(
    language: &str,
    role: PreviewRole,
    agent_label: &str,
    agent_color: Color,
    first: bool,
) -> Line<'static> {
    let mut spans = Vec::new();
    if role == PreviewRole::Assistant && first {
        spans.push(Span::styled(
            format!("{agent_label} "),
            Style::new().fg(agent_color).bold(),
        ));
    } else {
        spans.push(Span::styled(
            "  ".to_string(),
            Style::new().fg(Color::DarkGray),
        ));
    }
    spans.push(Span::styled(
        "```".to_string(),
        Style::new().fg(Color::DarkGray).italic(),
    ));
    if !language.is_empty() {
        spans.push(Span::styled(
            language.to_string(),
            Style::new().fg(Color::Rgb(120, 210, 255)).italic(),
        ));
    }
    Line::from(spans)
}

fn render_code_line(line: &str, language: &str, terms: &[String]) -> Line<'static> {
    let mut spans = vec![Span::styled(
        "    ".to_string(),
        Style::new().fg(Color::DarkGray),
    )];
    spans.extend(highlight_spans(code_spans(line, language), terms));
    Line::from(spans)
}

fn code_spans(line: &str, language: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut idx = 0usize;
    while idx < line.len() {
        let rest = &line[idx..];
        if rest.starts_with("//") || (hash_comments(language) && rest.starts_with('#')) {
            spans.push(Span::styled(
                rest.to_string(),
                Style::new().fg(Color::Rgb(100, 160, 120)).italic(),
            ));
            break;
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };
        if matches!(ch, '"' | '\'' | '`') {
            let end = string_end(rest, ch);
            spans.push(Span::styled(
                rest[..end].to_string(),
                Style::new().fg(Color::Rgb(150, 220, 150)),
            ));
            idx += end;
        } else if ch.is_ascii_digit() {
            let end = take_while_len(rest, |ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_')
            });
            spans.push(Span::styled(
                rest[..end].to_string(),
                Style::new().fg(Color::Rgb(210, 160, 255)),
            ));
            idx += end;
        } else if ch == '_' || ch.is_ascii_alphabetic() {
            let end = take_while_len(rest, |ch| ch == '_' || ch.is_ascii_alphanumeric());
            let word = &rest[..end];
            spans.push(Span::styled(word.to_string(), code_word_style(word)));
            idx += end;
        } else {
            let len = ch.len_utf8();
            let style = if ch.is_whitespace() {
                Style::new()
            } else {
                Style::new().fg(Color::DarkGray)
            };
            spans.push(Span::styled(ch.to_string(), style));
            idx += len;
        }
    }
    spans
}

fn hash_comments(language: &str) -> bool {
    matches!(
        language,
        "bash"
            | "fish"
            | "py"
            | "python"
            | "rb"
            | "ruby"
            | "sh"
            | "shell"
            | "toml"
            | "yaml"
            | "yml"
            | "zsh"
    )
}

fn code_word_style(word: &str) -> Style {
    if is_code_keyword(word) {
        Style::new().fg(Color::Rgb(120, 210, 255)).bold()
    } else if matches!(
        word,
        "true" | "false" | "null" | "None" | "Some" | "Ok" | "Err" | "True" | "False"
    ) {
        Style::new().fg(Color::Rgb(210, 160, 255))
    } else {
        Style::new().fg(Color::Rgb(220, 225, 230))
    }
}

fn is_code_keyword(word: &str) -> bool {
    matches!(
        word,
        "as" | "async"
            | "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "def"
            | "do"
            | "else"
            | "enum"
            | "except"
            | "export"
            | "finally"
            | "fn"
            | "for"
            | "from"
            | "function"
            | "if"
            | "impl"
            | "import"
            | "in"
            | "interface"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "then"
            | "trait"
            | "try"
            | "type"
            | "use"
            | "var"
            | "where"
            | "while"
            | "with"
            | "yield"
    )
}

fn string_end(value: &str, quote: char) -> usize {
    let mut escaped = false;
    for (idx, ch) in value.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return idx + ch.len_utf8();
        }
    }
    value.len()
}

fn take_while_len(value: &str, mut predicate: impl FnMut(char) -> bool) -> usize {
    value
        .char_indices()
        .find_map(|(idx, ch)| (!predicate(ch)).then_some(idx))
        .unwrap_or(value.len())
}

fn highlight_spans(spans: Vec<Span<'static>>, terms: &[String]) -> Vec<Span<'static>> {
    if terms.is_empty() {
        return spans;
    }

    let mut highlighted = Vec::new();
    for span in spans {
        let text = span.content.into_owned();
        let lower = text.to_ascii_lowercase();
        let mut idx = 0usize;
        while idx < text.len() {
            let next = terms
                .iter()
                .filter(|term| !term.is_empty())
                .filter_map(|term| lower[idx..].find(term).map(|pos| (idx + pos, term.len())))
                .min_by_key(|(pos, _)| *pos);
            let Some((hit, len)) = next else {
                highlighted.push(Span::styled(text[idx..].to_string(), span.style));
                break;
            };
            if hit > idx {
                highlighted.push(Span::styled(text[idx..hit].to_string(), span.style));
            }
            let end = (hit + len).min(text.len());
            highlighted.push(Span::styled(
                text[hit..end].to_string(),
                Style::new()
                    .fg(Color::Black)
                    .bg(Color::Rgb(250, 220, 110))
                    .bold(),
            ));
            idx = end;
        }
    }
    highlighted
}

fn preview_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .filter(|term| !is_search_filter_token(term))
        .map(|term| term.to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect()
}

fn is_search_filter_token(token: &str) -> bool {
    let token = token.strip_prefix('-').unwrap_or(token);
    ["agent:", "dir:", "date:"]
        .iter()
        .any(|prefix| token.starts_with(prefix))
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max).collect();
    out.push_str("\n...");
    out
}

#[cfg(test)]
mod tests {
    use chrono::Local;
    use ratatui::style::Modifier;

    use super::*;

    fn session_with_content(content: &str) -> Session {
        Session::new(
            "session",
            "codex",
            "Preview test",
            "/tmp/fast-resume",
            Local::now(),
            content,
            2,
        )
    }

    fn rendered_text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn preview_lines_render_roles_and_code_blocks() {
        let session = session_with_content(
            "» show rust\n\n  ```rust\n#[derive(Debug)]\nfn main() {\n    let answer = 42;\n}\n```\nLooks good",
        );

        let lines = render_preview_lines(&session, "");
        let rendered = rendered_text(&lines);

        assert!(rendered.contains("» show rust"));
        assert!(rendered.contains("codex ```rust"));
        assert!(rendered.contains("    fn main()"));
        assert!(rendered.contains("  Looks good"));

        let keyword = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref() == "fn")
            .expect("code keyword is highlighted");
        assert_eq!(keyword.style.fg, Some(Color::Rgb(120, 210, 255)));
        assert!(keyword.style.add_modifier.contains(Modifier::BOLD));

        let hash = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref() == "#")
            .expect("rust attributes are not comments");
        assert_eq!(hash.style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn preview_highlights_matches_inside_code_but_ignores_filter_tokens() {
        let session = session_with_content("» inspect\n\n  ```rust\nfn main() {}\n```");

        let lines = render_preview_lines(&session, "agent:codex main");

        let highlighted = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref() == "main")
            .expect("query term is highlighted in code");
        assert_eq!(highlighted.style.bg, Some(Color::Rgb(250, 220, 110)));

        assert!(
            lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .all(|span| span.content.as_ref() != "agent:codex")
        );
    }
}
