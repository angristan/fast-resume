use chrono::{DateTime, Local, TimeZone};
use tantivy::doc;
use tantivy::schema::{Field, TantivyDocument, Value};

use crate::model::Session;

use super::schema::IndexFields;

pub(super) fn session_document(fields: IndexFields, session: &Session) -> TantivyDocument {
    doc!(
        fields.id => session.id.clone(),
        fields.session_key => session_key(&session.agent, &session.id),
        fields.title => session.title.clone(),
        fields.directory => session.directory.clone(),
        fields.agent => session.agent.clone(),
        fields.content => session.content.clone(),
        fields.timestamp => datetime_to_seconds(session.timestamp),
        fields.message_count => session.message_count as i64,
        fields.mtime => session.mtime,
        fields.yolo => session.yolo,
        fields.named => session.named,
    )
}

pub(super) fn doc_to_session(fields: IndexFields, doc: &TantivyDocument) -> Option<Session> {
    let timestamp = number(doc, fields.timestamp)?;
    let mut session = Session::new(
        text(doc, fields.id)?.to_string(),
        text(doc, fields.agent).unwrap_or_default().to_string(),
        text(doc, fields.title).unwrap_or_default().to_string(),
        text(doc, fields.directory).unwrap_or_default().to_string(),
        Local.timestamp_opt(timestamp as i64, 0).single()?,
        text(doc, fields.content).unwrap_or_default().to_string(),
        integer(doc, fields.message_count).unwrap_or(0) as usize,
    );
    session.mtime = number(doc, fields.mtime).unwrap_or(0.0);
    session.yolo = boolean(doc, fields.yolo).unwrap_or(false);
    session.named = boolean(doc, fields.named).unwrap_or(false);
    Some(session)
}

pub(super) fn datetime_to_seconds(timestamp: DateTime<Local>) -> f64 {
    timestamp.timestamp() as f64 + f64::from(timestamp.timestamp_subsec_nanos()) / 1e9
}

pub(super) fn session_key(agent: &str, id: &str) -> String {
    format!("{agent}::{id}")
}

pub(super) fn text(doc: &TantivyDocument, field: Field) -> Option<&str> {
    doc.get_first(field).and_then(|value| value.as_str())
}

pub(super) fn number(doc: &TantivyDocument, field: Field) -> Option<f64> {
    doc.get_first(field).and_then(|value| value.as_f64())
}

fn integer(doc: &TantivyDocument, field: Field) -> Option<i64> {
    doc.get_first(field).and_then(|value| value.as_i64())
}

fn boolean(doc: &TantivyDocument, field: Field) -> Option<bool> {
    doc.get_first(field).and_then(|value| value.as_bool())
}
