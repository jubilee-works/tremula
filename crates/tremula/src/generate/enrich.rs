//! Turning what a model said into a mutant a manifest can carry.
//!
//! A model answers with the *text* to replace and never with a byte offset. That
//! is not a preference: asked for offsets, a measured sweep of 144 proposals got
//! them right in one case, and the text was findable in four out of five. So the
//! address is settled here, against the file's own bytes.
//!
//! The rule the search follows is the pack protocol's own: a function's mutation
//! targets are its `body_span` less the whole `span` of every function nested
//! inside it, less the stretches its report excludes. A text found anywhere else
//! is not found — a mutation of a nested function belongs to that function's own
//! round, and a mutation of a docstring is one no test suite could be blamed for
//! missing.
//!
//! And the text has to occur exactly *once*. Two occurrences are a defect the
//! model is told about, not a choice made on its behalf. The measured sweep's
//! search took the first of two matches five times; twice that put the text into a
//! parameter list, and one of those produced a function with two parameters of the
//! same name, which the interpreter refuses and which ended the run as an error.

use std::ops::Range;

use sha2::{Digest, Sha256};
use tremula_contracts::{
    manifest::{Mutant, Span},
    spans::FunctionSpan,
};

use crate::{
    generate::{Defect, GeneratedMutant},
    provenance,
    validation::canonical_mutant_id,
};

/// The proposal is about a file this round is not about.
pub const WRONG_FILE: &str = "wrong_file";

/// The text to replace is nowhere a mutation of this function may land.
pub const ORIGINAL_NOT_FOUND: &str = "original_not_found";

/// The text to replace is there more than once, so it addresses no one place.
pub const ORIGINAL_AMBIGUOUS: &str = "original_ambiguous";

/// The replacement is the text it replaces, so nothing would change.
pub const REPLACEMENT_UNCHANGED: &str = "replacement_unchanged";

/// The replacement holds a carriage return, which no manifest may carry.
pub const CARRIAGE_RETURN_IN_REPLACEMENT: &str = "carriage_return_in_replacement";

/// The replacement holds a NUL, which would make the identifier ambiguous.
pub const NUL_IN_REPLACEMENT: &str = "nul_in_replacement";

/// One source file, read once, as everything below measures it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// The file's path, POSIX-style and relative to the project root — the
    /// spelling a manifest uses for the same file.
    pub file: String,
    /// Its bytes, as they were read.
    pub bytes: Vec<u8>,
    /// SHA-256 of those bytes, lowercase hexadecimal. Every mutant derived from
    /// this file carries it, which is what makes a manifest go stale the moment
    /// the file changes.
    pub sha256: String,
}

impl SourceFile {
    /// Read a file's bytes as the file they are, hashing them once.
    #[must_use]
    pub fn new(file: impl Into<String>, bytes: Vec<u8>) -> Self {
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        Self {
            file: file.into(),
            bytes,
            sha256,
        }
    }

    /// The text one span of the file holds, or nothing when the span is not
    /// inside it or does not fall on character boundaries.
    #[must_use]
    pub fn text(&self, span: Span) -> Option<&str> {
        let range = extent(span);
        std::str::from_utf8(self.bytes.get(range)?).ok()
    }
}

/// One function as a place a mutation may land.
///
/// The searchable stretches are computed once, from the whole report rather than
/// from one entry: which bytes a function owns depends on what is nested inside
/// it, and that is only knowable from the other entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    function: FunctionSpan,
    regions: Vec<Range<usize>>,
}

impl Target {
    /// Work out where a mutation of `function` may land, given every function the
    /// same report names.
    ///
    /// A function nested inside this one is subtracted whole, because everything
    /// it is made of belongs to it. The stretches this function's own report
    /// excludes are subtracted too. What a nested function's *decorators* occupy
    /// belongs to neither, and no span in a report covers them — said plainly in
    /// the protocol rather than papered over, and caught by the same check that
    /// catches everything else: confirming what came back changes the function it
    /// claimed to change.
    #[must_use]
    pub fn new(function: &FunctionSpan, functions: &[FunctionSpan]) -> Self {
        let mut regions = vec![extent(function.body_span)];
        for other in functions {
            if nested_in(other, function) {
                regions = without(regions, &extent(other.span));
            }
        }
        for excluded in &function.excluded {
            regions = without(regions, &extent(excluded.span));
        }
        Self {
            function: function.clone(),
            regions,
        }
    }

    /// The function this target is about.
    #[must_use]
    pub fn function(&self) -> &FunctionSpan {
        &self.function
    }

    /// The stretches of the file a mutation of this function may aim at, in the
    /// order they appear.
    #[must_use]
    pub fn regions(&self) -> &[Range<usize>] {
        &self.regions
    }
}

/// What produced a mutant, as a manifest records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// The model that answered, spelled the way its provider names that exact
    /// snapshot — read from the answer rather than from what was asked for.
    pub model: String,
    /// When the answer came back, as RFC 3339 in UTC.
    pub generated_at: String,
}

/// Turn one proposal into a mutant of `file`, or say why it cannot be one.
///
/// # Errors
///
/// Returns the [`Defect`] that refuses it: a proposal about another file, a text
/// that is not in the searchable part of the function or is there twice, or a
/// replacement no manifest could carry. Every one of them is a defect a model can
/// be told about and asked to correct.
pub fn enrich(
    candidate: &GeneratedMutant,
    file: &SourceFile,
    target: &Target,
    stamp: &Stamp,
) -> Result<Mutant, Defect> {
    if candidate.file != file.file {
        return Err(defect(
            WRONG_FILE,
            format!(
                "this round is about `{}`, and the mutation names `{}`; answer about the file the request named",
                file.file, candidate.file
            ),
        ));
    }
    let span = the_one_occurrence(candidate, file, target)?;
    check_the_replacement(candidate)?;
    Ok(mutant(candidate, file, span, stamp))
}

/// The same mutant with the newline that closes its span taken in, if there is one.
///
/// A compound statement's node ends *after* the newline that closes it, so a
/// search that stopped before that newline found the right text at an extent no
/// backend can match. The newline goes into the span and into `original` together,
/// because every reader of a manifest relies on those being the same bytes.
///
/// The identifier is derived again, since a different span is a different mutation.
#[must_use]
pub fn with_the_closing_newline(mutant: &Mutant, file: &SourceFile) -> Option<Mutant> {
    let after = usize::try_from(mutant.span.end_byte).unwrap_or(usize::MAX);
    if file.bytes.get(after) != Some(&b'\n') {
        return None;
    }
    let span = Span {
        start_byte: mutant.span.start_byte,
        end_byte: mutant.span.end_byte + 1,
    };
    Some(Mutant {
        id: canonical_mutant_id(
            &mutant.file,
            &span,
            &mutant.base_file_sha256,
            &mutant.replacement,
        ),
        span,
        original: format!("{}\n", mutant.original),
        ..mutant.clone()
    })
}

/// Where the proposal's text sits, when it sits in exactly one place.
fn the_one_occurrence(
    candidate: &GeneratedMutant,
    file: &SourceFile,
    target: &Target,
) -> Result<Span, Defect> {
    if candidate.original.is_empty() {
        return Err(defect(
            ORIGINAL_NOT_FOUND,
            "the text to replace is empty, so it addresses nothing; quote the code to change"
                .to_owned(),
        ));
    }
    let found = occurrences(&file.bytes, candidate.original.as_bytes(), target.regions());
    match found.as_slice() {
        [only] => Ok(Span {
            start_byte: u64::try_from(only.start).unwrap_or(u64::MAX),
            end_byte: u64::try_from(only.end).unwrap_or(u64::MAX),
        }),
        [] => Err(defect(
            ORIGINAL_NOT_FOUND,
            format!(
                "`{}` does not occur in the body of `{}`; copy the text to change out of the function exactly as it is spelled there, and do not aim at a nested function or at anything the request excluded",
                candidate.original,
                target.function().qualified_name
            ),
        )),
        many => Err(defect(
            ORIGINAL_AMBIGUOUS,
            format!(
                "`{}` occurs {} times in the body of `{}`, so it does not say which one to change; widen it — a whole statement, say — until it occurs once",
                candidate.original,
                many.len(),
                target.function().qualified_name
            ),
        )),
    }
}

/// The rules a replacement has to keep whatever the file says about it.
///
/// Each of these would be refused later and worse. A carriage return is refused by
/// the neutral validation of the whole manifest, so one proposal would cost every
/// other mutant in it. A NUL is refused by nothing: the identifier is a hash over
/// parts joined with NUL bytes, so a NUL inside one of them lets two different
/// mutations derive the same identifier. And a replacement that is the text it
/// replaces changes nothing at all.
fn check_the_replacement(candidate: &GeneratedMutant) -> Result<(), Defect> {
    if candidate.replacement == candidate.original {
        return Err(defect(
            REPLACEMENT_UNCHANGED,
            "the replacement is the text it replaces, so the mutation changes nothing".to_owned(),
        ));
    }
    if candidate.replacement.contains('\r') {
        return Err(defect(
            CARRIAGE_RETURN_IN_REPLACEMENT,
            "the replacement holds a carriage return; write line feeds only".to_owned(),
        ));
    }
    if candidate.replacement.contains('\0') || candidate.original.contains('\0') {
        return Err(defect(
            NUL_IN_REPLACEMENT,
            "the replacement holds a NUL byte, which a mutant's identifier cannot be derived over"
                .to_owned(),
        ));
    }
    Ok(())
}

/// The mutant a settled span makes of a proposal.
fn mutant(candidate: &GeneratedMutant, file: &SourceFile, span: Span, stamp: &Stamp) -> Mutant {
    Mutant {
        id: canonical_mutant_id(&file.file, &span, &file.sha256, &candidate.replacement),
        file: file.file.clone(),
        base_file_sha256: file.sha256.clone(),
        span,
        original: candidate.original.clone(),
        replacement: candidate.replacement.clone(),
        description: (!candidate.description.is_empty()).then(|| candidate.description.clone()),
        provenance: provenance::generator(&stamp.model, &stamp.generated_at),
    }
}

fn defect(name: &str, detail: String) -> Defect {
    Defect {
        defect: name.to_owned(),
        detail,
    }
}

/// Every place `needle` occurs wholly inside one of `regions`.
///
/// Overlapping occurrences are all counted. Counting them once each would make
/// `aa` in `aaa` look like a text that says which one to change, and it does not.
fn occurrences(bytes: &[u8], needle: &[u8], regions: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    for region in regions {
        let Some(within) = bytes.get(region.clone()) else {
            continue;
        };
        let mut from = 0;
        while let Some(at) = starts_with_at(&within[from..], needle) {
            let start = region.start + from + at;
            found.push(start..start + needle.len());
            from += at + 1;
        }
    }
    found
}

/// The first offset into `haystack` at which `needle` starts.
fn starts_with_at(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&at| haystack[at..at + needle.len()] == *needle)
}

/// Whether `inner` is a function nested inside `outer` rather than `outer` itself.
fn nested_in(inner: &FunctionSpan, outer: &FunctionSpan) -> bool {
    inner.span != outer.span
        && inner.span.start_byte >= outer.span.start_byte
        && inner.span.end_byte <= outer.span.end_byte
}

/// The regions with `cut` taken out of every one it overlaps.
fn without(regions: Vec<Range<usize>>, cut: &Range<usize>) -> Vec<Range<usize>> {
    let mut left = Vec::with_capacity(regions.len());
    for region in regions {
        if cut.end <= region.start || cut.start >= region.end {
            left.push(region);
            continue;
        }
        if region.start < cut.start {
            left.push(region.start..cut.start);
        }
        if cut.end < region.end {
            left.push(cut.end..region.end);
        }
    }
    left
}

/// A span as a range of this host's own offsets.
///
/// A span that does not fit becomes one that reaches past every file, which is a
/// span nothing is found in — the honest reading of an offset this host cannot
/// address.
fn extent(span: Span) -> Range<usize> {
    usize::try_from(span.start_byte).unwrap_or(usize::MAX)
        ..usize::try_from(span.end_byte).unwrap_or(usize::MAX)
}
