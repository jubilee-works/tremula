//! The bridge between a coverage document, which counts lines, and a language pack,
//! which reports bytes.
//!
//! Two things are needed and only one of them is arithmetic. Turning a byte span into
//! lines is a table of newline offsets. Turning a changed line into the function it
//! belongs to is a decision, and one that a language pack's report cannot make on its
//! own: a pack places a function at its `def`, so the decorators above it are outside
//! the span it reports for it. A decorator is also executed when the module is
//! imported, so a coverage document says its line was reached. A pull request that
//! changed a route decorator therefore offers a changed, covered line that no
//! function's span contains — and a reader of the spans alone would select nothing.
//!
//! So a changed line inside the block of decorators directly above a function is
//! promoted to that function. The block is decorators and blank lines: those are what
//! stand between a decorator and the `def` it belongs to. Anything else ends the block,
//! and a class is not a function — so a `@dataclass` and the fields under it belong to
//! no function at all, and are counted rather than selected.

use tremula_contracts::{
    manifest::{LineRange, Span},
    spans::FunctionSpan,
};

/// One file's lines, as offsets into its bytes.
///
/// A file always has at least one line. A trailing newline ends the last line rather
/// than opening an empty one, which is what makes the count of lines the number a
/// person would say the file has.
#[derive(Debug)]
pub struct Lines<'a> {
    source: &'a [u8],
    /// Where each line begins, ascending.
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    /// The line table of one file's raw bytes.
    #[must_use]
    pub fn of(source: &'a [u8]) -> Self {
        let mut starts = vec![0usize];
        for (at, byte) in source.iter().enumerate() {
            if *byte == b'\n' && at + 1 < source.len() {
                starts.push(at + 1);
            }
        }
        Self { source, starts }
    }

    /// How many lines the file has.
    #[must_use]
    pub fn count(&self) -> u32 {
        u32::try_from(self.starts.len()).unwrap_or(u32::MAX)
    }

    /// The 1-indexed line a byte offset falls on.
    ///
    /// A newline belongs to the line it ends, and an offset past the end of the file
    /// is the last line: both of those are asked for by a span whose exclusive end is
    /// the file's length.
    #[must_use]
    pub fn at(&self, byte: u64) -> u32 {
        let byte = usize::try_from(byte).unwrap_or(usize::MAX);
        let found = self.starts.partition_point(|start| *start <= byte);
        u32::try_from(found.max(1)).unwrap_or(u32::MAX)
    }

    /// Where one line's first byte is. The end of the file for a line it does not have.
    #[must_use]
    pub fn start_of(&self, line: u32) -> u64 {
        let index = usize::try_from(line.saturating_sub(1)).unwrap_or(usize::MAX);
        let start = self.starts.get(index).copied().unwrap_or(self.source.len());
        u64::try_from(start).unwrap_or(u64::MAX)
    }

    /// One line's bytes, without the newline that ends it.
    #[must_use]
    pub fn line(&self, line: u32) -> &'a [u8] {
        let index = usize::try_from(line.saturating_sub(1)).unwrap_or(usize::MAX);
        let Some(start) = self.starts.get(index).copied() else {
            return &[];
        };
        let end = self
            .starts
            .get(index + 1)
            .copied()
            .unwrap_or(self.source.len());
        let text = self.source.get(start..end).unwrap_or_default();
        let text = text.strip_suffix(b"\n").unwrap_or(text);
        text.strip_suffix(b"\r").unwrap_or(text)
    }

    /// The lines a byte span covers, 1-indexed and inclusive at both ends.
    ///
    /// The span's end is exclusive, so the line of the last byte it holds is the answer:
    /// a span that ends at a newline would otherwise report the line after the last one
    /// it covers, and that line is code the span says nothing about.
    #[must_use]
    pub fn range_of(&self, span: Span) -> LineRange {
        let last = span.end_byte.max(span.start_byte.saturating_add(1)) - 1;
        LineRange {
            start_line: self.at(span.start_byte),
            end_line: self.at(last),
        }
    }
}

/// The function a changed line belongs to, or nothing when it belongs to none.
///
/// Innermost first, because a nested function's body is inside its parent's and it is
/// the nested one a mutation there changes. Then the decorators: a line the spans place
/// in no function, sitting in the block of decorators directly above one, belongs to
/// the function it decorates.
#[must_use]
pub fn owner<'report>(
    functions: &'report [FunctionSpan],
    lines: &Lines<'_>,
    line: u32,
) -> Option<&'report FunctionSpan> {
    innermost(functions, lines, line).or_else(|| decorated(functions, lines, line))
}

/// The innermost function whose lines hold this one.
fn innermost<'report>(
    functions: &'report [FunctionSpan],
    lines: &Lines<'_>,
    line: u32,
) -> Option<&'report FunctionSpan> {
    functions
        .iter()
        .filter(|function| {
            let extent = lines.range_of(function.span);
            extent.start_line <= line && line <= extent.end_line
        })
        .max_by_key(|function| function.span.start_byte)
}

/// The function a decorator line decorates.
///
/// Only from a decorator, and only down through decorators and blank lines to a line a
/// function begins on. A search that went further would attach a stray decorator at the
/// foot of a file to whatever function came next, and a search that stopped at anything
/// but a function's own first line would attach a class's decorator to a method of it.
fn decorated<'report>(
    functions: &'report [FunctionSpan],
    lines: &Lines<'_>,
    line: u32,
) -> Option<&'report FunctionSpan> {
    if !decorates(lines.line(line)) {
        return None;
    }
    let mut below = line.saturating_add(1);
    while below <= lines.count() {
        let text = lines.line(below);
        if blank(text) || decorates(text) {
            below = below.saturating_add(1);
            continue;
        }
        return functions
            .iter()
            .find(|function| lines.range_of(function.span).start_line == below);
    }
    None
}

/// Whether a line is a decorator.
fn decorates(text: &[u8]) -> bool {
    trimmed(text).first() == Some(&b'@')
}

/// Whether a line holds nothing but space.
fn blank(text: &[u8]) -> bool {
    trimmed(text).is_empty()
}

/// A line without the space it begins with.
fn trimmed(text: &[u8]) -> &[u8] {
    let at = text
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(text.len());
    text.get(at..).unwrap_or_default()
}
