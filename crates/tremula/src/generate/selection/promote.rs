//! From the lines a change touched to the functions worth asking about, and the order a
//! limit cuts them in.

use std::collections::BTreeMap;

use tremula_contracts::{
    manifest::{LineRange, Span},
    spans::SpansReport,
};

use crate::generate::selection::lines::{Lines, owner};

/// One function a change touched, before any limit is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The file it is in, POSIX-style and relative to the project root.
    pub file: String,
    /// The function, as the language pack names it.
    pub function: String,
    /// Where the function is, which is what identifies it.
    pub span: Span,
    /// The same extent as lines, for a person to read.
    pub lines: LineRange,
    /// How many of the change's lines fell inside it.
    pub candidate_lines: u32,
}

/// What one file's changed lines came to.
#[derive(Debug, Default)]
pub struct Promoted {
    /// The functions those lines belong to, in the order the file spells them.
    pub candidates: Vec<Candidate>,
    /// How many of the lines belonged to no function at all.
    pub outside: u32,
}

/// Which functions of one file a change touched.
///
/// A line that belongs to no function is counted and not otherwise recorded: what a
/// reader does about a changed module-level assignment or a changed class field is the
/// same whichever line it was, and listing them would bury the functions that were
/// selected under the ones that could not be.
#[must_use]
pub fn candidates(
    file: &str,
    report: &SpansReport,
    lines: &Lines<'_>,
    touched: &[u32],
) -> Promoted {
    let mut counted: BTreeMap<(u64, u64), Candidate> = BTreeMap::new();
    let mut outside = 0u32;
    for line in touched {
        let Some(function) = owner(&report.functions, lines, *line) else {
            outside = outside.saturating_add(1);
            continue;
        };
        let key = (function.span.start_byte, function.span.end_byte);
        let already = counted.entry(key).or_insert_with(|| Candidate {
            file: file.to_owned(),
            function: function.qualified_name.clone(),
            span: function.span,
            lines: lines.range_of(function.span),
            candidate_lines: 0,
        });
        already.candidate_lines = already.candidate_lines.saturating_add(1);
    }
    Promoted {
        candidates: counted.into_values().collect(),
        outside,
    }
}

/// The candidates in the order a limit cuts them.
///
/// Most touched first, because that is where a change is concentrated and where a
/// mutation is most likely to be about what somebody just wrote. Then the path, then
/// where in the file the function is — neither of which is a judgement about value, and
/// both of which are there so that the same change selects the same functions on every
/// machine and in every run. A limit that cut in an unsettled order would make two runs
/// of one commit disagree about what was tested.
#[must_use]
pub fn ordered(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut ordered = candidates;
    ordered.sort_by(|one, other| {
        other
            .candidate_lines
            .cmp(&one.candidate_lines)
            .then_with(|| one.file.cmp(&other.file))
            .then_with(|| one.span.start_byte.cmp(&other.span.start_byte))
    });
    ordered
}
