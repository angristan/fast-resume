use std::ops::Bound;

use anyhow::Result;
use chrono::{DateTime, Days, TimeZone};
use tantivy::query::{
    AllQuery, BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, QueryParser, RangeQuery,
    RegexQuery, TermQuery, TermSetQuery,
};
use tantivy::schema::IndexRecordOption;
use tantivy::{Index, Term};

use crate::query::{DateFilter, DateOp, Filter};

use super::document::datetime_to_seconds;
use super::schema::IndexFields;

pub(super) fn build(
    index: &Index,
    fields: IndexFields,
    search_text: &str,
    agent_filter: Option<Filter>,
    directory_filter: Option<Filter>,
    date_filter: Option<DateFilter>,
) -> Result<Box<dyn Query>> {
    let mut parts: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    if !search_text.is_empty() {
        parts.push((Occur::Must, text_query(index, fields, search_text)?));
    }

    if let Some(query) = agent_query(fields, agent_filter) {
        parts.push((Occur::Must, query));
    }
    if let Some(query) = directory_query(fields, directory_filter)? {
        parts.push((Occur::Must, query));
    }
    if let Some(date) = date_filter {
        let query = date_query(fields, &date);
        if date.negated {
            if parts.is_empty() {
                parts.push((Occur::Must, Box::new(AllQuery)));
            }
            parts.push((Occur::MustNot, query));
        } else {
            parts.push((Occur::Must, query));
        }
    }

    Ok(if parts.is_empty() {
        Box::new(AllQuery)
    } else {
        Box::new(BooleanQuery::new(parts))
    })
}

fn text_query(index: &Index, fields: IndexFields, search_text: &str) -> Result<Box<dyn Query>> {
    let parser =
        QueryParser::for_index(index, vec![fields.title, fields.content, fields.directory]);
    let (exact, _) = parser.parse_query_lenient(search_text);
    let boosted_exact = BoostQuery::new(exact, 5.0);

    let mut alternatives: Vec<(Occur, Box<dyn Query>)> =
        vec![(Occur::Should, Box::new(boosted_exact))];
    let fuzzy_parts: Vec<(Occur, Box<dyn Query>)> = search_text
        .split_whitespace()
        .filter(|term| term.chars().count() >= 3)
        .map(|term| {
            let term = term.to_lowercase();
            let title =
                FuzzyTermQuery::new_prefix(Term::from_field_text(fields.title, &term), 1, true);
            let content =
                FuzzyTermQuery::new_prefix(Term::from_field_text(fields.content, &term), 1, true);
            let fields = BooleanQuery::new(vec![
                (Occur::Should, Box::new(title) as Box<dyn Query>),
                (Occur::Should, Box::new(content) as Box<dyn Query>),
            ]);
            (Occur::Must, Box::new(fields) as Box<dyn Query>)
        })
        .collect();

    if !fuzzy_parts.is_empty() {
        alternatives.push((Occur::Should, Box::new(BooleanQuery::new(fuzzy_parts))));
    }
    if let Some(directory_query) = directory_text_query(fields, search_text)? {
        alternatives.push((Occur::Should, directory_query));
    }

    Ok(Box::new(BooleanQuery::new(alternatives)))
}

fn directory_text_query(fields: IndexFields, search_text: &str) -> Result<Option<Box<dyn Query>>> {
    let search_text = search_text.trim();
    if search_text.chars().count() < 3 || search_text.split_whitespace().count() != 1 {
        return Ok(None);
    }
    let pattern = format!("(?i).*{}.*", regex::escape(search_text));
    Ok(Some(Box::new(RegexQuery::from_pattern(
        &pattern,
        fields.directory,
    )?)))
}

fn agent_query(fields: IndexFields, filter: Option<Filter>) -> Option<Box<dyn Query>> {
    let filter = filter?;
    let mut parts: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    if !filter.include.is_empty() {
        let terms = filter
            .include
            .iter()
            .map(|agent| Term::from_field_text(fields.agent, agent));
        parts.push((Occur::Must, Box::new(TermSetQuery::new(terms))));
    }
    for excluded in filter.exclude {
        let term = Term::from_field_text(fields.agent, &excluded);
        parts.push((
            Occur::MustNot,
            Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
        ));
    }
    if parts.is_empty() {
        return None;
    }
    if parts.iter().all(|(occur, _)| *occur == Occur::MustNot) {
        parts.insert(0, (Occur::Must, Box::new(AllQuery)));
    }
    Some(Box::new(BooleanQuery::new(parts)))
}

fn directory_query(fields: IndexFields, filter: Option<Filter>) -> Result<Option<Box<dyn Query>>> {
    let Some(filter) = filter else {
        return Ok(None);
    };
    let mut parts: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    if !filter.include.is_empty() {
        let include_parts: Result<Vec<_>> = filter
            .include
            .iter()
            .map(|dir| {
                let pattern = format!("(?i).*{}.*", regex::escape(dir));
                Ok((
                    Occur::Should,
                    Box::new(RegexQuery::from_pattern(&pattern, fields.directory)?)
                        as Box<dyn Query>,
                ))
            })
            .collect();
        parts.push((Occur::Must, Box::new(BooleanQuery::new(include_parts?))));
    }

    for excluded in filter.exclude {
        let pattern = format!("(?i).*{}.*", regex::escape(&excluded));
        let query = RegexQuery::from_pattern(&pattern, fields.directory)?;
        parts.push((Occur::MustNot, Box::new(query)));
    }
    if parts.is_empty() {
        return Ok(None);
    }
    if parts.iter().all(|(occur, _)| *occur == Occur::MustNot) {
        parts.insert(0, (Occur::Must, Box::new(AllQuery)));
    }
    Ok(Some(Box::new(BooleanQuery::new(parts))))
}

fn date_query(fields: IndexFields, date: &DateFilter) -> Box<dyn Query> {
    let cutoff = datetime_to_seconds(date.cutoff);
    match date.op {
        DateOp::LessThan => Box::new(RangeQuery::new(
            Bound::Included(Term::from_field_f64(fields.timestamp, cutoff)),
            Bound::Unbounded,
        )),
        DateOp::GreaterThan => Box::new(RangeQuery::new(
            Bound::Unbounded,
            Bound::Excluded(Term::from_field_f64(fields.timestamp, cutoff)),
        )),
        DateOp::Exact if date.value.eq_ignore_ascii_case("today") => Box::new(RangeQuery::new(
            Bound::Included(Term::from_field_f64(fields.timestamp, cutoff)),
            Bound::Unbounded,
        )),
        DateOp::Exact if date.value.eq_ignore_ascii_case("yesterday") => {
            let end = next_day_start_seconds(&date.cutoff).unwrap_or(cutoff + 86_400.0);
            Box::new(RangeQuery::new(
                Bound::Included(Term::from_field_f64(fields.timestamp, cutoff)),
                Bound::Excluded(Term::from_field_f64(fields.timestamp, end)),
            ))
        }
        DateOp::Exact => Box::new(AllQuery),
    }
}

fn next_day_start_seconds<Tz: TimeZone>(cutoff: &DateTime<Tz>) -> Option<f64> {
    cutoff
        .clone()
        .checked_add_days(Days::new(1))
        .map(|end| end.timestamp_millis() as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use chrono_tz::Europe::Paris;

    use super::next_day_start_seconds;

    #[test]
    fn next_day_start_follows_civil_time_across_dst() {
        let spring = Paris
            .with_ymd_and_hms(2026, 3, 29, 0, 0, 0)
            .single()
            .unwrap();
        let fall = Paris
            .with_ymd_and_hms(2026, 10, 25, 0, 0, 0)
            .single()
            .unwrap();

        assert_eq!(
            next_day_start_seconds(&spring).unwrap() - spring.timestamp() as f64,
            82_800.0
        );
        assert_eq!(
            next_day_start_seconds(&fall).unwrap() - fall.timestamp() as f64,
            90_000.0
        );
    }
}
