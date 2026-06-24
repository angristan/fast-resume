use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tantivy::schema::{
    Field, IndexRecordOption, NumericOptions, STORED, Schema, TEXT, TextFieldIndexing, TextOptions,
};

use crate::config::INDEX_SCHEMA_VERSION;

const VERSION_FILE: &str = ".schema_version";

#[derive(Debug, Clone, Copy)]
pub(super) struct IndexFields {
    pub(super) id: Field,
    pub(super) session_key: Field,
    pub(super) title: Field,
    pub(super) directory: Field,
    pub(super) agent: Field,
    pub(super) content: Field,
    pub(super) timestamp: Field,
    pub(super) message_count: Field,
    pub(super) mtime: Field,
    pub(super) yolo: Field,
}

impl IndexFields {
    pub(super) fn from_schema(schema: &Schema) -> Result<Self> {
        Ok(Self {
            id: schema.get_field("id")?,
            session_key: schema.get_field("session_key")?,
            title: schema.get_field("title")?,
            directory: schema.get_field("directory")?,
            agent: schema.get_field("agent")?,
            content: schema.get_field("content")?,
            timestamp: schema.get_field("timestamp")?,
            message_count: schema.get_field("message_count")?,
            mtime: schema.get_field("mtime")?,
            yolo: schema.get_field("yolo")?,
        })
    }
}

pub(super) fn build_schema() -> Schema {
    let mut schema = Schema::builder();
    schema.add_text_field("id", raw_text_options());
    schema.add_text_field("session_key", raw_text_options());
    schema.add_text_field("title", TEXT | STORED);
    schema.add_text_field("directory", raw_text_options());
    schema.add_text_field("agent", raw_text_options());
    schema.add_text_field("content", TEXT | STORED);
    schema.add_f64_field(
        "timestamp",
        NumericOptions::default()
            .set_stored()
            .set_indexed()
            .set_fast(),
    );
    schema.add_i64_field("message_count", STORED);
    schema.add_f64_field("mtime", STORED);
    schema.add_bool_field("yolo", STORED);
    schema.build()
}

pub(super) fn schema_version_matches(path: &Path) -> bool {
    let version_file = path.join(VERSION_FILE);
    fs::read_to_string(version_file)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        == Some(INDEX_SCHEMA_VERSION)
}

pub(super) fn write_schema_version(path: &Path) -> Result<()> {
    fs::write(path.join(VERSION_FILE), INDEX_SCHEMA_VERSION.to_string())
        .with_context(|| format!("failed to write schema version in {}", path.display()))
}

fn raw_text_options() -> TextOptions {
    TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("raw")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    )
}
