//! Custom harnesses declared in the user config file.
//!
//! A custom harness describes a JSONL session store with a field mapping, so
//! fast-resume can index an agent that has no built-in adapter. See the
//! "Custom harnesses" section of `docs/usage.md` for the file format.

use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use ratatui::style::Color;
use serde::Deserialize;
use serde_json::Value;

use crate::config::{BUILTIN_AGENTS, expand_tilde, home_dir};

/// Placeholders replaced in resume commands.
const ID_PLACEHOLDER: &str = "{id}";
const DIRECTORY_PLACEHOLDER: &str = "{directory}";

const DEFAULT_COLOR: Color = Color::Rgb(160, 160, 160);
const DEFAULT_LIGHT_COLOR: Color = Color::Rgb(90, 90, 90);

static LOADED: Lazy<Loaded> = Lazy::new(|| {
    // Unit tests must not read the developer's config file.
    if cfg!(test) {
        return Loaded::default();
    }
    match load_file(&config_file()) {
        Ok(harnesses) => Loaded {
            harnesses,
            error: None,
        },
        Err(error) => Loaded {
            harnesses: Vec::new(),
            error: Some(error),
        },
    }
});

#[derive(Default)]
struct Loaded {
    harnesses: Vec<Harness>,
    error: Option<String>,
}

/// The custom harnesses from the config file, in file order. Empty when the
/// file is missing or invalid.
pub fn harnesses() -> &'static [Harness] {
    &LOADED.harnesses
}

/// Why the config file could not be used, if it could not.
pub fn load_error() -> Option<&'static str> {
    LOADED.error.as_deref()
}

/// The config file: `$FAST_RESUME_CONFIG`, else
/// `$XDG_CONFIG_HOME/fast-resume/config.toml` (when absolute), else
/// `~/.config/fast-resume/config.toml`.
pub fn config_file() -> PathBuf {
    if let Some(path) = env::var_os("FAST_RESUME_CONFIG").filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("fast-resume")
        .join("config.toml")
}

fn load_file(path: &Path) -> Result<Vec<Harness>, String> {
    match fs::read_to_string(path) {
        Ok(text) => parse_config(&text).map_err(|error| format!("{}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Parse and validate a config file's contents.
pub fn parse_config(text: &str) -> Result<Vec<Harness>, String> {
    let file: ConfigFile = toml::from_str(text).map_err(|error| error.to_string())?;
    let mut names = HashSet::new();
    file.harness
        .into_iter()
        .map(|raw| {
            let name = raw.name.clone();
            let harness =
                Harness::try_from(raw).map_err(|error| format!("harness {name:?}: {error}"))?;
            if !names.insert(harness.name.clone()) {
                return Err(format!("harness {name:?} is declared twice"));
            }
            Ok(harness)
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    harness: Vec<RawHarness>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHarness {
    name: String,
    badge: Option<String>,
    color: Option<String>,
    light_color: Option<String>,
    dir: String,
    #[serde(default = "default_extension")]
    extension: String,
    #[serde(default)]
    recursive: bool,
    resume: Vec<String>,
    yolo_resume: Option<Vec<String>>,
    format: RawFormat,
}

fn default_extension() -> String {
    "jsonl".to_string()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFormat {
    kind: String,
    timestamp: Option<String>,
    rows: BTreeMap<String, RawRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRow {
    id: Option<String>,
    cwd: Option<String>,
    title: Option<String>,
    timestamp: Option<String>,
    role: Option<String>,
    text: Option<String>,
    roles: Option<BTreeMap<String, String>>,
}

/// A validated custom harness.
#[derive(Debug, Clone)]
pub struct Harness {
    pub name: String,
    pub badge: String,
    pub color: Color,
    pub light_color: Color,
    pub dir: PathBuf,
    pub extension: String,
    pub recursive: bool,
    pub resume: Vec<String>,
    pub yolo_resume: Option<Vec<String>>,
    pub format: Format,
}

impl Harness {
    /// The resume command for a session, with placeholders filled in.
    pub fn resume_command(&self, id: &str, directory: &str, yolo: bool) -> Vec<String> {
        let template = match (&self.yolo_resume, yolo) {
            (Some(yolo_resume), true) => yolo_resume,
            _ => &self.resume,
        };
        template
            .iter()
            .map(|arg| {
                arg.replace(ID_PLACEHOLDER, id)
                    .replace(DIRECTORY_PLACEHOLDER, directory)
            })
            .collect()
    }
}

impl TryFrom<RawHarness> for Harness {
    type Error = String;

    fn try_from(raw: RawHarness) -> Result<Self, String> {
        validate_name(&raw.name)?;
        let badge = raw.badge.unwrap_or_else(|| raw.name.clone());
        if badge.trim().is_empty() {
            return Err("badge must not be empty".to_string());
        }
        let color = raw.color.as_deref().map(parse_color).transpose()?;
        let light_color = raw.light_color.as_deref().map(parse_color).transpose()?;
        if raw.dir.trim().is_empty() {
            return Err("dir must not be empty".to_string());
        }
        validate_command("resume", &raw.resume)?;
        if let Some(yolo_resume) = &raw.yolo_resume {
            validate_command("yolo_resume", yolo_resume)?;
        }
        Ok(Self {
            name: raw.name,
            badge,
            color: color.unwrap_or(DEFAULT_COLOR),
            light_color: light_color.or(color).unwrap_or(DEFAULT_LIGHT_COLOR),
            dir: expand_tilde(&raw.dir),
            extension: raw.extension.trim_start_matches('.').to_string(),
            recursive: raw.recursive,
            resume: raw.resume,
            yolo_resume: raw.yolo_resume,
            format: Format::try_from(raw.format)?,
        })
    }
}

/// Names are used in `agent:` filters and `--agent`, so they are restricted to
/// lowercase letters, digits, and dashes, and must not shadow a built-in agent.
fn validate_name(name: &str) -> Result<(), String> {
    let valid = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err("name must use lowercase letters, digits, and dashes".to_string());
    }
    if BUILTIN_AGENTS.contains(&name) {
        return Err("name is already used by a built-in agent".to_string());
    }
    Ok(())
}

fn validate_command(key: &str, command: &[String]) -> Result<(), String> {
    if command
        .first()
        .is_none_or(|program| program.trim().is_empty())
    {
        return Err(format!("{key} must start with a program"));
    }
    if !command.iter().any(|arg| arg.contains(ID_PLACEHOLDER)) {
        return Err(format!("{key} must contain {ID_PLACEHOLDER}"));
    }
    Ok(())
}

fn parse_color(value: &str) -> Result<Color, String> {
    let hex = value
        .strip_prefix('#')
        .filter(|hex| hex.len() == 6)
        .ok_or_else(|| format!("color {value:?} must look like \"#rrggbb\""))?;
    let channel = |range| {
        u8::from_str_radix(&hex[range], 16)
            .map_err(|_| format!("color {value:?} must look like \"#rrggbb\""))
    };
    Ok(Color::Rgb(channel(0..2)?, channel(2..4)?, channel(4..6)?))
}

/// How rows of a session file map to session fields.
#[derive(Debug, Clone)]
pub struct Format {
    pub kind: FieldPath,
    pub timestamp: Option<FieldPath>,
    pub rows: BTreeMap<String, RowMapping>,
}

impl TryFrom<RawFormat> for Format {
    type Error = String;

    fn try_from(raw: RawFormat) -> Result<Self, String> {
        if raw.rows.is_empty() {
            return Err("format.rows must map at least one row kind".to_string());
        }
        let rows = raw
            .rows
            .into_iter()
            .map(|(kind, row)| {
                let mapping = RowMapping::try_from(row)
                    .map_err(|error| format!("format.rows.{kind}: {error}"))?;
                Ok((kind, mapping))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            kind: FieldPath::parse(&raw.kind).map_err(|error| format!("format.kind: {error}"))?,
            timestamp: optional_path(raw.timestamp.as_deref())
                .map_err(|error| format!("format.timestamp: {error}"))?,
            rows,
        })
    }
}

/// The fields read from one kind of row.
#[derive(Debug, Clone, Default)]
pub struct RowMapping {
    pub id: Option<FieldPath>,
    pub cwd: Option<FieldPath>,
    pub title: Option<FieldPath>,
    pub timestamp: Option<FieldPath>,
    pub role: Option<FieldPath>,
    pub text: Option<FieldPath>,
    /// Source role value to `user` or `assistant`. Unlisted roles are skipped.
    pub roles: BTreeMap<String, Role>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl TryFrom<RawRow> for RowMapping {
    type Error = String;

    fn try_from(raw: RawRow) -> Result<Self, String> {
        if raw.roles.is_some() && raw.role.is_none() {
            return Err("roles needs role".to_string());
        }
        let roles = match raw.roles {
            Some(roles) => roles
                .into_iter()
                .map(|(source, role)| match role.as_str() {
                    "user" => Ok((source, Role::User)),
                    "assistant" => Ok((source, Role::Assistant)),
                    _ => Err(format!(
                        "roles.{source} must be \"user\" or \"assistant\", not {role:?}"
                    )),
                })
                .collect::<Result<_, String>>()?,
            None => BTreeMap::from([
                ("user".to_string(), Role::User),
                ("assistant".to_string(), Role::Assistant),
            ]),
        };
        let path = |key: &str, value: Option<&str>| {
            optional_path(value).map_err(|error| format!("{key}: {error}"))
        };
        Ok(Self {
            id: path("id", raw.id.as_deref())?,
            cwd: path("cwd", raw.cwd.as_deref())?,
            title: path("title", raw.title.as_deref())?,
            timestamp: path("timestamp", raw.timestamp.as_deref())?,
            role: path("role", raw.role.as_deref())?,
            text: path("text", raw.text.as_deref())?,
            roles,
        })
    }
}

fn optional_path(value: Option<&str>) -> Result<Option<FieldPath>, String> {
    value.map(FieldPath::parse).transpose()
}

/// A path into a JSON row, such as `message.content[type=text].text`.
///
/// Segments are object keys separated by dots. Arrays are walked element by
/// element. `key[field=a|b]` keeps only the elements of the `key` array whose
/// `field` equals one of the listed values; when `key` holds a plain string
/// instead of an array, that string is the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldPath {
    source: String,
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    key: String,
    filter: Option<(String, Vec<String>)>,
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

impl FieldPath {
    pub fn parse(source: &str) -> Result<Self, String> {
        let invalid = |reason: &str| format!("invalid path {source:?}: {reason}");
        let mut segments = Vec::new();
        let mut rest = source;
        loop {
            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            let key = &rest[..end];
            if key.is_empty() {
                return Err(invalid("empty key"));
            }
            rest = &rest[end..];
            let mut filter = None;
            if let Some(after) = rest.strip_prefix('[') {
                let close = after.find(']').ok_or_else(|| invalid("missing ]"))?;
                let (field, values) = after[..close]
                    .split_once('=')
                    .ok_or_else(|| invalid("filters look like [field=value]"))?;
                let values: Vec<String> = values.split('|').map(str::to_string).collect();
                if field.is_empty() || values.iter().any(String::is_empty) {
                    return Err(invalid("empty filter field or value"));
                }
                filter = Some((field.to_string(), values));
                rest = &after[close + 1..];
            }
            segments.push(Segment {
                key: key.to_string(),
                filter,
            });
            if rest.is_empty() {
                break;
            }
            rest = rest
                .strip_prefix('.')
                .ok_or_else(|| invalid("expected . after ]"))?;
        }
        Ok(Self {
            source: source.to_string(),
            segments,
        })
    }

    /// Every value the path reaches, in document order.
    pub fn values<'a>(&self, row: &'a Value) -> Vec<&'a Value> {
        let mut out = Vec::new();
        walk(row, &self.segments, &mut out);
        out
    }

    /// The first string the path reaches, or a number as a string.
    pub fn string(&self, row: &Value) -> Option<String> {
        self.values(row).into_iter().find_map(|value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
    }

    /// Every non-empty string the path reaches.
    pub fn texts<'a>(&self, row: &'a Value) -> Vec<&'a str> {
        self.values(row)
            .into_iter()
            .filter_map(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .collect()
    }
}

fn walk<'a>(value: &'a Value, segments: &[Segment], out: &mut Vec<&'a Value>) {
    if let Value::Array(items) = value {
        for item in items {
            walk(item, segments, out);
        }
        return;
    }
    let Some((segment, rest)) = segments.split_first() else {
        out.push(value);
        return;
    };
    let Some(child) = value.get(&segment.key) else {
        return;
    };
    let Some((field, accepted)) = &segment.filter else {
        walk(child, rest, out);
        return;
    };
    match child {
        Value::Array(items) => {
            for item in items {
                let matches = item
                    .get(field)
                    .and_then(Value::as_str)
                    .is_some_and(|value| accepted.iter().any(|accepted| accepted == value));
                if matches {
                    walk(item, rest, out);
                }
            }
        }
        Value::String(_) => out.push(child),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const PITI: &str = r#"
[[harness]]
name = "piti"
dir = "~/.local/share/piti/sessions"
resume = ["piti", "--resume", "{id}"]
yolo_resume = ["piti", "--yes-all", "--resume", "{id}"]

[harness.format]
kind = "type"
timestamp = "timestamp"

[harness.format.rows.session]
id = "id"
cwd = "cwd"

[harness.format.rows.title]
title = "title"

[harness.format.rows.message]
role = "message.role"
text = "message.content[type=text].text"
"#;

    #[test]
    fn parses_a_harness() {
        let harnesses = parse_config(PITI).expect("valid config");
        let [harness] = harnesses.as_slice() else {
            panic!("one harness");
        };
        assert_eq!(harness.name, "piti");
        assert_eq!(harness.badge, "piti");
        assert_eq!(harness.dir, home_dir().join(".local/share/piti/sessions"));
        assert_eq!(harness.extension, "jsonl");
        assert!(!harness.recursive);
        assert_eq!(harness.format.rows.len(), 3);
        let message = &harness.format.rows["message"];
        assert_eq!(message.roles["user"], Role::User);
        assert_eq!(message.roles["assistant"], Role::Assistant);
    }

    #[test]
    fn fills_resume_placeholders() {
        let harnesses = parse_config(PITI).expect("valid config");
        let harness = &harnesses[0];
        assert_eq!(
            harness.resume_command("abc", "/repo", false),
            ["piti", "--resume", "abc"]
        );
        assert_eq!(
            harness.resume_command("abc", "/repo", true),
            ["piti", "--yes-all", "--resume", "abc"]
        );
    }

    #[test]
    fn empty_config_has_no_harnesses() {
        assert!(parse_config("").expect("valid config").is_empty());
    }

    #[test]
    fn rejects_invalid_harnesses() {
        let cases = [
            (PITI.replace("\"piti\"\n", "\"Piti\"\n"), "lowercase"),
            (PITI.replace("\"piti\"\n", "\"claude\"\n"), "built-in"),
            (
                PITI.replace("\"--resume\", \"{id}\"]\nyolo", "\"--resume\"]\nyolo"),
                "{id}",
            ),
            (
                PITI.replace("kind = \"type\"", "kind = \"a..b\""),
                "empty key",
            ),
            (PITI.replace("[type=text]", "[type]"), "[field=value]"),
            (PITI.replace("id = \"id\"", "ide = \"id\""), "unknown field"),
            (format!("{PITI}{PITI}"), "declared twice"),
            (format!("{PITI}color = \"red\"\n"), "unknown field"),
        ];
        for (config, expected) in cases {
            let error = parse_config(&config).expect_err(expected);
            assert!(
                error.contains(expected),
                "{error:?} should mention {expected:?}"
            );
        }
    }

    #[test]
    fn parses_colors() {
        assert_eq!(parse_color("#ff8000"), Ok(Color::Rgb(255, 128, 0)));
        assert!(parse_color("ff8000").is_err());
        assert!(parse_color("#ff80zz").is_err());
    }

    #[test]
    fn walks_nested_paths_and_filters() {
        let row = json!({
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "first"},
                    {"type": "thinking", "thinking": "hidden"},
                    {"type": "tool_call", "text": "not text"},
                    {"type": "text", "text": "second"}
                ]
            }
        });
        let text = FieldPath::parse("message.content[type=text].text").expect("path");
        assert_eq!(text.texts(&row), ["first", "second"]);
        let both = FieldPath::parse("message.content[type=text|tool_call].text").expect("path");
        assert_eq!(both.texts(&row), ["first", "not text", "second"]);
        let role = FieldPath::parse("message.role").expect("path");
        assert_eq!(role.string(&row).as_deref(), Some("assistant"));
        assert!(
            FieldPath::parse("missing")
                .expect("path")
                .values(&row)
                .is_empty()
        );
    }

    #[test]
    fn filtered_string_content_is_the_text() {
        let row = json!({"message": {"content": "plain prompt"}});
        let text = FieldPath::parse("message.content[type=text].text").expect("path");
        assert_eq!(text.texts(&row), ["plain prompt"]);
    }

    #[test]
    fn numbers_read_as_strings() {
        let row = json!({"id": 42});
        let id = FieldPath::parse("id").expect("path");
        assert_eq!(id.string(&row).as_deref(), Some("42"));
    }

    #[test]
    fn rejects_malformed_paths() {
        for path in ["", ".a", "a.", "a[b=c", "a[=c]", "a[b=]", "a[b=c]d"] {
            assert!(
                FieldPath::parse(path).is_err(),
                "{path:?} should be rejected"
            );
        }
    }
}
