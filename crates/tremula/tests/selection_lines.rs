//! Turning byte offsets into lines, and turning a changed line into the function it
//! belongs to.
//!
//! The second of those is where a decorated function is won or lost. A language pack
//! places a function at its `def`, so its decorators are outside the span it reports
//! — and a decorator line is executed when the module is imported, so coverage says
//! it was reached. A pull request that changed only a route decorator therefore has a
//! changed, covered line that no function's span contains, and reading it literally
//! would select nothing at all.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use tremula_contracts::{manifest::Span, spans::FunctionSpan};

use tremula::generate::selection::lines::{Lines, owner};

/// The functions of `source`, found the crude way a test may: every `def` line, out
/// to the end of its indented block. The real spans come from the language pack, and
/// what is under test here is what is done with them rather than how they are found.
fn functions(source: &str) -> Vec<FunctionSpan> {
    let bytes = source.as_bytes();
    let lines = Lines::of(bytes);
    let mut found = Vec::new();
    for line in 1..=lines.count() {
        let text = String::from_utf8_lossy(lines.line(line)).into_owned();
        let trimmed = text.trim_start();
        if !trimmed.starts_with("def ") && !trimmed.starts_with("async def ") {
            continue;
        }
        let indent = text.len() - trimmed.len();
        let mut last = line;
        for below in line + 1..=lines.count() {
            let text = String::from_utf8_lossy(lines.line(below)).into_owned();
            if text.trim().is_empty() {
                continue;
            }
            if text.len() - text.trim_start().len() <= indent {
                break;
            }
            last = below;
        }
        let start = lines.start_of(line);
        let end = lines.start_of(last) + u64::try_from(lines.line(last).len()).unwrap();
        let name = trimmed
            .trim_start_matches("async ")
            .trim_start_matches("def ")
            .split('(')
            .next()
            .unwrap()
            .to_owned();
        found.push(FunctionSpan {
            qualified_name: name,
            span: Span {
                start_byte: start,
                end_byte: end,
            },
            body_span: Span {
                start_byte: start,
                end_byte: end,
            },
            excluded: Vec::new(),
        });
    }
    found
}

/// Which function a line belongs to, by name, or nothing when it belongs to none.
fn owner_of(source: &str, line: u32) -> Option<String> {
    let found = functions(source);
    owner(&found, &Lines::of(source.as_bytes()), line)
        .map(|function| function.qualified_name.clone())
}

#[test]
fn a_byte_offset_is_the_line_it_falls_on() {
    let source = "one\ntwo\nthree\n";
    let lines = Lines::of(source.as_bytes());

    assert_eq!(lines.count(), 3);
    assert_eq!(lines.at(0), 1);
    assert_eq!(lines.at(3), 1, "the newline belongs to the line it ends");
    assert_eq!(lines.at(4), 2);
    assert_eq!(lines.at(13), 3);
}

/// A file whose last line has no newline still has that line, and an offset past the
/// end of the file is the last line rather than a line the file does not have.
#[test]
fn a_last_line_without_a_newline_is_still_a_line() {
    let lines = Lines::of(b"one\ntwo");

    assert_eq!(lines.count(), 2);
    assert_eq!(lines.at(6), 2);
    assert_eq!(lines.at(7), 2);
    assert_eq!(lines.at(9000), 2, "past the end is the end");
}

#[test]
fn an_empty_file_has_one_empty_line() {
    let lines = Lines::of(b"");

    assert_eq!(lines.count(), 1);
    assert_eq!(lines.at(0), 1);
}

/// The lines a span covers, 1-indexed and inclusive at both ends. The exclusive end
/// of a byte span lands on the line after the last one when the span ends at a
/// newline, and reporting that line would name code the span does not hold.
#[test]
fn a_span_becomes_the_lines_it_covers() {
    let source = "def f():\n    return 1\n\ndef g():\n    return 2\n";
    let lines = Lines::of(source.as_bytes());

    let whole = lines.range_of(Span {
        start_byte: 0,
        end_byte: 22,
    });

    assert_eq!(whole.start_line, 1);
    assert_eq!(whole.end_line, 2);
    let one_line = lines.range_of(Span {
        start_byte: 9,
        end_byte: 21,
    });
    assert_eq!((one_line.start_line, one_line.end_line), (2, 2));
}

#[test]
fn a_line_inside_a_function_belongs_to_that_function() {
    let source = "def overlaps(a, b):\n    return a < b\n";

    assert_eq!(owner_of(source, 2).as_deref(), Some("overlaps"));
    assert_eq!(owner_of(source, 1).as_deref(), Some("overlaps"));
}

/// The innermost one, because a nested function's body is inside its parent's and it
/// is the nested one whose behaviour a mutation there changes.
#[test]
fn a_line_inside_a_nested_function_belongs_to_the_inner_one() {
    let source = "def outer():\n    def inner():\n        return 1\n    return inner\n";

    assert_eq!(owner_of(source, 3).as_deref(), Some("inner"));
    assert_eq!(owner_of(source, 4).as_deref(), Some("outer"));
}

#[test]
fn a_decorator_line_belongs_to_the_function_it_decorates() {
    let source = "@app.route(\"/x\")\ndef view():\n    return 1\n";

    assert_eq!(
        owner_of(source, 1).as_deref(),
        Some("view"),
        "a changed route is a changed function"
    );
}

#[test]
fn every_decorator_of_a_stack_belongs_to_the_function_below_it() {
    let source = "@app.route(\"/x\")\n@login_required\n@cached\ndef view():\n    return 1\n";

    for line in 1..=3 {
        assert_eq!(
            owner_of(source, line).as_deref(),
            Some("view"),
            "line {line}"
        );
    }
}

/// A blank line between the decorators, or between the last of them and the `def`,
/// is legal Python and does not detach one from the other.
#[test]
fn a_blank_line_among_the_decorators_does_not_detach_them() {
    let source = "@app.route(\"/x\")\n\n@login_required\n\ndef view():\n    return 1\n";

    assert_eq!(owner_of(source, 1).as_deref(), Some("view"));
    assert_eq!(owner_of(source, 3).as_deref(), Some("view"));
}

/// A method's decorators reach the method, which is the case that made this rule
/// worth having: a project whose routes are methods has every one of them here.
#[test]
fn a_decorator_inside_a_class_belongs_to_the_method_below_it() {
    let source = "class View:\n    @property\n    def name(self):\n        return 1\n";

    assert_eq!(owner_of(source, 2).as_deref(), Some("name"));
}

/// Where the lookback stops. A class is not a function, so a decorator of one belongs
/// to no function and is counted as a line outside them all — as is everything in a
/// class body, including the fields a `@dataclass` is made of.
#[test]
fn a_decorated_class_and_its_fields_belong_to_no_function() {
    let source =
        "@dataclass\nclass Event:\n    start: int\n    end: int\n\ndef f():\n    return 1\n";

    assert_eq!(owner_of(source, 1), None, "the class's own decorator");
    assert_eq!(owner_of(source, 2), None, "the class statement");
    assert_eq!(owner_of(source, 3), None, "a field of it");
    assert_eq!(owner_of(source, 4), None);
}

#[test]
fn a_line_that_is_not_a_decorator_and_is_in_no_function_belongs_to_none() {
    let source = "TIMEOUT = 30\n\ndef f():\n    return TIMEOUT\n";

    assert_eq!(owner_of(source, 1), None);
    assert_eq!(owner_of(source, 2), None);
}

/// A decorator with nothing under it — the last lines of a file somebody is in the
/// middle of writing — belongs to nothing, and looking further would attach it to
/// whatever function came next in the file.
#[test]
fn a_decorator_with_no_function_under_it_belongs_to_none() {
    let source = "def f():\n    return 1\n\n@app.route(\"/x\")\n";

    assert_eq!(owner_of(source, 4), None);
}

/// A comment between a decorator and its function is legal, and is not a decorator:
/// the rule reaches a contiguous block of decorators and says so, rather than
/// scanning downwards until it finds something it likes.
#[test]
fn a_comment_between_a_decorator_and_its_function_stops_the_lookback() {
    let source = "@app.route(\"/x\")\n# why this route exists\ndef view():\n    return 1\n";

    assert_eq!(owner_of(source, 1), None);
    assert_eq!(owner_of(source, 2), None);
}
