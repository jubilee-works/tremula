//! Which function a caller meant.
//!
//! A qualified name is for a person to read and is never a key: a file can spell
//! one twice — a definition made twice under different conditions, a property's
//! getter and setter, an overload — and the language pack reports the same name for
//! each of them. What identifies a function is its own byte span, so a name that is
//! not unique is refused with the spellings that would each name exactly one, and
//! the caller says which by adding the span to the name.
//!
//! Nothing here guesses. A single match is the answer; none and more than one are
//! both failures, and each says what the file does have.

use tremula_contracts::{
    manifest::Span,
    spans::{FunctionSpan, SpansReport},
};

use crate::generate::failures::GenerationFailure;

/// What separates a function's name from the span that disambiguates it.
pub(crate) const AT: char = '@';

/// The functions the caller named, in the order they were named.
///
/// # Errors
///
/// Returns [`GenerationFailure`] for a name the file does not have, a name it
/// spells more than once, or a disambiguator that is not a span.
pub(crate) fn choose<'report>(
    report: &'report SpansReport,
    named: &[String],
) -> Result<Vec<&'report FunctionSpan>, GenerationFailure> {
    named
        .iter()
        .map(|spelling| one_of(report, spelling))
        .collect()
}

/// The one function a name — or a name and a span — picks out.
fn one_of<'report>(
    report: &'report SpansReport,
    spelling: &str,
) -> Result<&'report FunctionSpan, GenerationFailure> {
    let (name, at) = match spelling.split_once(AT) {
        None => (spelling, None),
        Some((name, extent)) => (name, Some(span_from(spelling, extent)?)),
    };
    let matching: Vec<&FunctionSpan> = report
        .functions
        .iter()
        .filter(|function| function.qualified_name == name)
        .filter(|function| at.is_none_or(|span| function.span == span))
        .collect();
    match matching.as_slice() {
        [only] => Ok(only),
        [] => Err(GenerationFailure::NoSuchFunction {
            name: name.to_owned(),
            file: report.file.clone(),
            available: listed(report.functions.iter().map(|function| {
                format!(
                    "`{}{AT}{}:{}`",
                    function.qualified_name, function.span.start_byte, function.span.end_byte
                )
            })),
        }),
        many => Err(GenerationFailure::AmbiguousFunction {
            name: name.to_owned(),
            file: report.file.clone(),
            count: many.len(),
            choices: listed(many.iter().map(|function| {
                format!(
                    "`{name}{AT}{}:{}`",
                    function.span.start_byte, function.span.end_byte
                )
            })),
        }),
    }
}

/// The span a disambiguator spells.
fn span_from(spelling: &str, extent: &str) -> Result<Span, GenerationFailure> {
    let unreadable = || GenerationFailure::UnreadableDisambiguator {
        spelling: spelling.to_owned(),
    };
    let (start, end) = extent.split_once(':').ok_or_else(unreadable)?;
    Ok(Span {
        start_byte: start.parse().map_err(|_| unreadable())?,
        end_byte: end.parse().map_err(|_| unreadable())?,
    })
}

fn listed(items: impl Iterator<Item = String>) -> String {
    let listed: Vec<String> = items.collect();
    if listed.is_empty() {
        return "none".to_owned();
    }
    listed.join(", ")
}
