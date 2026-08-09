//! The schema an enum gets when a newer producer may extend it.
//!
//! `#[derive(JsonSchema)]` writes an enum as a choice between the constants it
//! declares, which is right for an enum whose value set is closed and wrong for
//! one carrying an `Unknown` fallback. The fallback is a promise that a consumer
//! reads a value invented after it was built; a schema listing only today's
//! constants breaks that promise for every consumer that validates a document
//! before parsing it, because the validator refuses what the parser would have
//! accepted.

use schemars::Schema;
use serde_json::Value;

/// What the description says before it lists the values.
const TOLERANCE: &str = "Any string is valid here. A consumer reads a value this \
    contract version does not define as `unknown` rather than refusing the document \
    that carries it, so a validator must not refuse it either. The values this \
    version does define:";

/// Relax a derived enum schema to accept any string.
///
/// Meant for the `transform` argument of `#[schemars(...)]` on an enum with a
/// `#[serde(other)]` variant. The constants stop being a constraint and become
/// part of the description, each with the sentence that documents it, so a reader
/// of the schema alone still learns what this version emits — while a producer
/// that emits more than that no longer contradicts the published contract.
pub(crate) fn accepts_any_string(schema: &mut Schema) {
    let listed = listed_values(schema.remove("oneOf"));
    schema.remove("enum");
    schema.insert("type".to_owned(), Value::String("string".to_owned()));
    let existing = schema
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let described = if existing.is_empty() {
        format!("{TOLERANCE}\n{listed}")
    } else {
        format!("{existing}\n\n{TOLERANCE}\n{listed}")
    };
    schema.insert("description".to_owned(), Value::String(described));
}

/// The constants of a derived enum schema, as a list a person can read.
///
/// A variant the derive documented arrives with a description of its own, which
/// is the only account of what the value means and so is kept. Anything the
/// derive spelled some other way contributes nothing rather than a half-entry:
/// the constraint has already been relaxed by the time this is called, so a value
/// missing from the list costs a reader a line of documentation and no more.
fn listed_values(one_of: Option<Value>) -> String {
    let Some(Value::Array(variants)) = one_of else {
        return String::new();
    };
    let mut lines: Vec<String> = Vec::new();
    for variant in &variants {
        let Some(name) = variant.get("const").and_then(Value::as_str) else {
            continue;
        };
        let documented = variant
            .get("description")
            .and_then(Value::as_str)
            .map(|doc| format!(" — {}", on_one_line(doc)))
            .unwrap_or_default();
        lines.push(format!("- `{name}`{documented}"));
    }
    lines.join("\n")
}

/// A doc comment as a single line, so that the list reads as a list.
fn on_one_line(doc: &str) -> String {
    doc.split_whitespace().collect::<Vec<_>>().join(" ")
}
