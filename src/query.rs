use chrono::{DateTime, Duration, Local, NaiveTime};
use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOp {
    Exact,
    LessThan,
    GreaterThan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DateFilter {
    pub op: DateOp,
    pub value: String,
    pub cutoff: DateTime<Local>,
    pub negated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Filter {
    pub fn values(&self) -> Vec<&str> {
        self.include
            .iter()
            .chain(self.exclude.iter())
            .map(String::as_str)
            .collect()
    }

    pub fn negated(&self) -> bool {
        self.include.is_empty() && !self.exclude.is_empty()
    }

    pub fn matches(&self, value: &str, substring: bool) -> bool {
        if self.include.is_empty() && self.exclude.is_empty() {
            return true;
        }

        let matches_one = |needle: &str| {
            if substring {
                value.to_lowercase().contains(&needle.to_lowercase())
            } else {
                value == needle
            }
        };

        if self.exclude.iter().any(|value| matches_one(value)) {
            return false;
        }
        self.include.is_empty() || self.include.iter().any(|value| matches_one(value))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedQuery {
    pub text: String,
    pub agent: Option<Filter>,
    pub directory: Option<Filter>,
    pub date: Option<DateFilter>,
}

static KEYWORD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(-?)(agent|dir|date):(?:"([^"]+)"|(\S+))"#).expect("valid regex"));

static RELATIVE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([<>])?(\d+)(m|h|d|w|mo|y)$").expect("valid regex"));

pub fn parse_query(query: &str) -> ParsedQuery {
    let mut agent = None;
    let mut directory = None;
    let mut date = None;

    for caps in KEYWORD_RE.captures_iter(query) {
        let negated = caps.get(1).map(|m| m.as_str()) == Some("-");
        let keyword = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let value = caps
            .get(3)
            .or_else(|| caps.get(4))
            .map(|m| m.as_str())
            .unwrap_or_default();

        match keyword {
            "agent" => agent = Some(parse_filter_value(value, negated)),
            "dir" => directory = Some(parse_filter_value(value, negated)),
            "date" => date = parse_date_value(value, negated),
            _ => {}
        }
    }

    let text = KEYWORD_RE.replace_all(query, "");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");

    ParsedQuery {
        text,
        agent,
        directory,
        date,
    }
}

fn parse_filter_value(value: &str, negated: bool) -> Filter {
    let mut filter = Filter::default();
    for raw in value.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        if let Some(value) = raw.strip_prefix('!') {
            filter.exclude.push(value.to_string());
        } else if negated {
            filter.exclude.push(raw.to_string());
        } else {
            filter.include.push(raw.to_string());
        }
    }
    filter
}

fn parse_date_value(value: &str, mut negated: bool) -> Option<DateFilter> {
    let value = if let Some(stripped) = value.strip_prefix('!') {
        negated = true;
        stripped
    } else {
        value
    };

    let now = Local::now();
    let lower = value.to_lowercase();
    match lower.as_str() {
        "today" => {
            let cutoff = now
                .date_naive()
                .and_time(NaiveTime::MIN)
                .and_local_timezone(Local)
                .earliest()?;
            Some(DateFilter {
                op: DateOp::Exact,
                value: value.to_string(),
                cutoff,
                negated,
            })
        }
        "yesterday" => {
            let cutoff_date = now.date_naive() - Duration::days(1);
            let cutoff = cutoff_date
                .and_time(NaiveTime::MIN)
                .and_local_timezone(Local)
                .earliest()?;
            Some(DateFilter {
                op: DateOp::Exact,
                value: value.to_string(),
                cutoff,
                negated,
            })
        }
        "week" => Some(DateFilter {
            op: DateOp::LessThan,
            value: value.to_string(),
            cutoff: now - Duration::days(7),
            negated,
        }),
        "month" => Some(DateFilter {
            op: DateOp::LessThan,
            value: value.to_string(),
            cutoff: now - Duration::days(30),
            negated,
        }),
        _ => parse_relative_time(value, now, negated),
    }
}

fn parse_relative_time(value: &str, now: DateTime<Local>, negated: bool) -> Option<DateFilter> {
    let lower = value.to_lowercase();
    let caps = RELATIVE_RE.captures(&lower)?;
    let op = caps.get(1).map(|m| m.as_str()).unwrap_or("<");
    let number: i64 = caps.get(2)?.as_str().parse().ok()?;
    let unit = caps.get(3)?.as_str();
    let seconds = match unit {
        "m" => number * 60,
        "h" => number * 60 * 60,
        "d" => number * 24 * 60 * 60,
        "w" => number * 7 * 24 * 60 * 60,
        "mo" => number * 30 * 24 * 60 * 60,
        "y" => number * 365 * 24 * 60 * 60,
        _ => return None,
    };

    Some(DateFilter {
        op: if op == ">" {
            DateOp::GreaterThan
        } else {
            DateOp::LessThan
        },
        value: value.to_string(),
        cutoff: now - Duration::seconds(seconds),
        negated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_query() {
        let parsed = parse_query("api auth bug");
        assert_eq!(parsed.text, "api auth bug");
        assert!(parsed.agent.is_none());
    }

    #[test]
    fn parses_agent_and_dir_keywords() {
        let parsed = parse_query(r#"agent:claude dir:"my project" auth"#);
        assert_eq!(parsed.text, "auth");
        assert_eq!(parsed.agent.unwrap().include, vec!["claude"]);
        assert_eq!(parsed.directory.unwrap().include, vec!["my project"]);
    }

    #[test]
    fn parses_mixed_negation() {
        let parsed = parse_query("agent:claude,!codex -dir:test api");
        let agent = parsed.agent.unwrap();
        assert_eq!(agent.include, vec!["claude"]);
        assert_eq!(agent.exclude, vec!["codex"]);
        let dir = parsed.directory.unwrap();
        assert_eq!(dir.exclude, vec!["test"]);
        assert_eq!(parsed.text, "api");
    }

    #[test]
    fn parses_date_aliases() {
        assert!(parse_query("date:today").date.is_some());
        assert!(parse_query("-date:<2d").date.unwrap().negated);
        assert_eq!(
            parse_query("date:>1w").date.unwrap().op,
            DateOp::GreaterThan
        );
    }
}
