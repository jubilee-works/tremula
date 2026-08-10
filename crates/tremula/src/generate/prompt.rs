//! What a model is told, and the archive of what it was told when this was
//! measured.
//!
//! Two prompts live here. [`archive`] holds the prompt of the measurement, kept
//! verbatim and never sent, so that the version below can be read as a delta
//! against something real instead of against memory. [`SYSTEM`] and [`assemble`]
//! are what is actually sent.

use crate::generate::{Defect, GenerationRequest};

/// Which revision of the prompt below produced an answer.
///
/// Goes into a mutant's `provenance.generator.prompt_version`, so that a change
/// in what the mutants look like can be traced to a change in what was asked.
pub const PROMPT_VERSION: &str = "1";

/// The prompt of the measurement, kept as the record and never sent.
///
/// Everything here was measured against `gpt-5.2-2025-12-11`: 48 calls, 12
/// functions, every answer schema-compliant and none needing a retry. Two facts
/// are worth more than the text itself. The first is that asking for byte
/// offsets does not work — under one per cent of the spans came back usable,
/// against four out of five for the text of `original`, which is why [`SYSTEM`]
/// asks for text. The second is [`ANTI_EQUIVALENCE`], which was an attempt to
/// stop the model proposing mutations that cannot change what a function does:
/// it was measured too, it did not work — every one of the four sampled
/// witnesses it produced was fabricated rather than separating — and it made the
/// answers worse elsewhere. It is kept so that the next person to have the idea
/// can read the result instead of repeating the experiment.
pub mod archive {
    /// The version this archive records.
    pub const VERSION: &str = "0";

    /// The system prompt, exactly as it was sent.
    pub const SYSTEM: &str =
        "You plant realistic bugs in Python code. Given one function, you produce
mutations that a competent developer could plausibly have written by mistake and
that an existing test suite might not catch.

Every mutation replaces exactly one complete syntax node inside the given
function, and obeys these rules:

- `span` is a half-open byte range into the whole file: 0-indexed, end
  exclusive, counted in UTF-8 bytes.
- `original` is exactly the text those bytes hold.
- `replacement` is valid Python that parses on its own as a single node, with no
  comment and no blank line around it.
- The span is never empty. A pure insertion cannot be expressed.
- Never change only formatting or comments: a mutation whose syntax tree matches
  the original teaches nothing and is discarded.
- Never add an import, and never touch test code.
- `description` says which real mistake the mutation simulates and why a test
  might miss it.

Aim at the faults the suite would let through, not at obvious breakage. A
mutation that stops the module from importing is worthless.";

    /// The user prompt, as the template it was assembled from.
    ///
    /// One test file is shown; the measured run repeated that block for each
    /// file that covered the function.
    pub const USER_TEMPLATE: &str = "file: {file}
This function occupies bytes {start} to {end} of that file, end exclusive. Byte {start} is the `d` of its `def`. Offsets you count inside the snippet below are relative to the snippet, so add {start} to each one.

```python
{source}
```

These are the tests that cover it. A mutation they already catch teaches nothing.

file: {test_file}
```python
{test_source}
```

Produce {count} mutations of the function above.";

    /// The block that was appended to [`SYSTEM`] to try to stop the model
    /// proposing mutations that cannot change what the function does.
    ///
    /// It did not work, and this is the record of that. It named the six
    /// families of equivalent mutation that had been observed and demanded a
    /// separating input for each proposal; the answers still contained
    /// equivalent mutations, the separating inputs it wrote down were fabricated,
    /// and the share of mutants that survived the test suite fell by more than
    /// twenty points. Whatever discards an equivalent mutation, it is not this.
    pub const ANTI_EQUIVALENCE: &str = r#"
A mutation that cannot change what the function *does* is worthless, and this is
the way most attempts fail. So, before proposing one, name the input that
separates it from the original: concrete argument values, reachable through the
function's own guards, for which the mutated function returns a different value,
raises a different exception, or writes something different. If you cannot name
such an input, do not propose that mutation — propose another one.

These are refused outright, because none of them can change an observable result:

- changing only a type annotation, a docstring, a comment, or formatting. Nothing
  evaluates an annotation at run time.
- an algebraically identical rewrite. `-(-a // b)` and `(a + b - 1) // b` are the
  same function for positive `b`; `timedelta(days=1)` equals `timedelta(hours=24)`;
  `str(s)` is `s` when `s` is already a `str`; a `.copy()` of a value nobody
  mutates is that value.
- a difference an earlier guard in the same function makes unreachable. If
  `if len(items) > 1: return ...` runs above, then `items[0]` and `items[-1]` are
  the same element. If a constructor already refused a missing key, `d["k"]` and
  `d.get("k")` are the same lookup.
- a normalization that recomputes what the value already was: appending a trailing
  newline before splitting on newlines, stripping what was already stripped.

`description` must state the separating input and both results, in this form:
"for <arguments>, the original <result> and the mutant <result>"."#;
}

/// The system prompt this version sends.
///
/// What this version says to a model differs from the measurement in five places
/// and no others, because the measurement stands only for the text it was taken
/// against. Three of the five are lines of the text below, against
/// [`archive::SYSTEM`]:
///
/// 1. **No span.** The two rules that required or described the `span` field are
///    gone, along with the paragraph of the user prompt that stated the
///    function's byte offsets. There is no span in the answer, so a prompt that
///    asked for one was asking for a field nobody would read; the emptiness rule
///    those lines also carried is implied by change 3, which asks for text that
///    can be found.
/// 2. **No standalone-parse requirement.** `replacement` no longer has to
///    "parse on its own as a single node". Judging a replacement that way
///    refused nearly a quarter of the measured answers, and splicing it into the
///    file and compiling that accepted eighteen of them — so the clause is gone
///    and the file decides. The rest of the rule, which keeps a comment and a
///    blank line out of a replacement, stays.
/// 3. **`original` is text to find, not bytes to hold.** It is now verbatim
///    contiguous text that occurs exactly once inside the function's body, which
///    is what a caller searching for it needs it to be.
///
/// Every other sentence of this prompt, and the position of every blank line, is
/// the archive's. The remaining two changes are not in this text at all:
///
/// 4. **The correction turn says "the contract", not "the schema".** One word, in
///    the message that goes back with a defect list — see [`correction`], which
///    says why: the list now also carries defects found after the schema was
///    satisfied. It is counted here because it is a change to what a model is
///    told, and the whole of what a model is told is what the measurement was
///    taken against.
/// 5. **The user prompt can name stretches not to aim at.** A block listing the
///    code a mutation must leave alone, present only when
///    [`GenerationRequest::excluded`] holds something — so a request without one
///    sends the measured prompt exactly. It is there because the caller refuses
///    such a mutation in any case, and a refusal costs one of the corrections a
///    round owns; saying so up front is the cheaper half of the same rule. What it
///    does *not* attempt is talking a model out of proposing mutations that cannot
///    change what a function does — that was measured, it did not work, and
///    [`ANTI_EQUIVALENCE`] is the record of it.
///
/// [`ANTI_EQUIVALENCE`]: archive::ANTI_EQUIVALENCE
pub const SYSTEM: &str = "You plant realistic bugs in Python code. Given one function, you produce
mutations that a competent developer could plausibly have written by mistake and
that an existing test suite might not catch.

Every mutation replaces exactly one complete syntax node inside the given
function, and obeys these rules:

- `original` is verbatim contiguous text that can be found exactly once inside
  the function's body.
- `replacement` is valid Python, with no comment and no blank line around it.
- Never change only formatting or comments: a mutation whose syntax tree matches
  the original teaches nothing and is discarded.
- Never add an import, and never touch test code.
- `description` says which real mistake the mutation simulates and why a test
  might miss it.

Aim at the faults the suite would let through, not at obvious breakage. A
mutation that stops the module from importing is worthless.";

/// What one call says to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// The standing instructions: [`SYSTEM`].
    pub system: String,
    /// The function, its tests, and how many mutations to produce.
    pub user: String,
    /// What was wrong with the last answer, when there was one. A provider sends
    /// it as a further turn rather than folding it into `user`, so that the
    /// question and the correction stay legible as two things.
    pub correction: Option<String>,
}

/// Assemble the prompt for one request.
///
/// The blank lines are load-bearing — the measured prompt built its parts as a
/// list and joined them with newlines, so an empty part is what put a blank line
/// between two blocks, and each one is reproduced here rather than left to
/// chance. A trailing newline on any source text is trimmed for the same reason:
/// the fence has to sit against the last line of code.
#[must_use]
pub fn assemble(request: &GenerationRequest) -> Prompt {
    let mut parts: Vec<String> = vec![
        format!("file: {}", request.file),
        String::new(),
        "```python".to_owned(),
        request.source.trim_end_matches('\n').to_owned(),
        "```".to_owned(),
    ];
    if !request.tests.is_empty() {
        parts.push(String::new());
        parts.push(
            "These are the tests that cover it. A mutation they already catch teaches nothing."
                .to_owned(),
        );
        for test in &request.tests {
            parts.push(String::new());
            parts.push(format!("file: {}", test.file));
            parts.push("```python".to_owned());
            parts.push(test.source.trim_end_matches('\n').to_owned());
            parts.push("```".to_owned());
        }
    }
    if !request.excluded.is_empty() {
        parts.push(String::new());
        parts.push(
            "Leave these alone. Nothing evaluates them, so a mutation of one changes nothing a test could see."
                .to_owned(),
        );
        for left_alone in &request.excluded {
            parts.push(String::new());
            parts.push("```python".to_owned());
            parts.push(left_alone.trim_end_matches('\n').to_owned());
            parts.push("```".to_owned());
        }
    }
    parts.push(String::new());
    // The sentence keeps the measured wording for every count, including one:
    // a prompt whose grammar changes with a number is a prompt nobody measured.
    parts.push(format!(
        "Produce {} mutations of the function above.",
        request.mutant_count
    ));
    Prompt {
        system: SYSTEM.to_owned(),
        user: parts.join("\n"),
        correction: request
            .feedback
            .as_ref()
            .map(|feedback| correction(&feedback.defects)),
    }
}

/// The turn that goes back with a defect list.
///
/// The wording is the measured one, with a single word changed: the message says
/// "the contract" rather than "the schema", because the same message now also
/// carries defects found after the schema was satisfied — text that is not in
/// the function, text that is there twice, a replacement that will not compile.
/// That word is the fourth of the changes [`SYSTEM`] enumerates, counted there so
/// that one list holds every difference from the measurement.
#[must_use]
pub fn correction(defects: &[Defect]) -> String {
    // A list of two strings cannot fail to serialise; an empty list would still
    // be a message rather than a panic.
    let listed = serde_json::to_string(defects).unwrap_or_else(|_| "[]".to_owned());
    format!(
        "That response did not satisfy the contract. Problems: {listed} Answer again, with the same JSON object shape and nothing else."
    )
}
