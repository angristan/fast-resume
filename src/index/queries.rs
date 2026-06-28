use std::ops::Bound;

use anyhow::Result;
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

    Ok(Box::new(BooleanQuery::new(alternatives)))
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
            let end = cutoff + 86_400.0;
            Box::new(RangeQuery::new(
                Bound::Included(Term::from_field_f64(fields.timestamp, cutoff)),
                Bound::Excluded(Term::from_field_f64(fields.timestamp, end)),
            ))
        }
        DateOp::Exact => Box::new(AllQuery),
    }
}
