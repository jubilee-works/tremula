//! What a model is asked about one mutation.
//!
//! A second prompt rather than a setting on the first. The generating prompt asks
//! for plausible mistakes and is measured for that; this one asks a question with
//! a right answer that can be checked by running something, and it says so to the
//! model. The two would drift apart under any shared wording, and the measurement
//! of either would stop meaning anything.

use crate::generate::{
    Defect,
    judge::JudgementRequest,
    prompt::{Prompt, correction},
};

/// Which revision of the prompt below produced a judgement.
///
/// Goes into `triage.json`, so a change in how survivors are sorted can be traced
/// to a change in what was asked.
pub const PROMPT_VERSION: &str = "1";

/// The standing instructions of a judgement.
///
/// Three things it does, and one it refuses to do. It asks for a claim, because a
/// claim is what selects the next step. It asks for the input the claim stands on,
/// in the form a probe can evaluate — a call of the function with literal
/// arguments — because that is the part with any standing. It says the input will
/// be executed, which is the only sentence that makes a fabricated one costly.
///
/// What it refuses to do is bargain for a verdict. Nothing here tells a model that
/// answering `equivalent` will retire the mutation, and nothing offers it the
/// benefit of the doubt, because neither is true: an answer of `equivalent` sends
/// the survivor to a person, and no answer of any kind is trusted without a
/// witness that ran.
pub const SYSTEM: &str =
    "You are given one Python function, a piece of text inside it, and the text
somebody proposes to put in its place. You decide whether that change can change
what the function does.

Answer `claim` with one of two values:

- `distinguishable`: there is an input for which the changed function returns a
  different value, raises a different exception, or prints something different.
- `equivalent`: there is no such input.

When you claim `distinguishable`, name that input in `witness`.

- `call` is one call of the function, by the name its `def` gives it, and every
  argument is a literal: a number, a string, a boolean, `None`, or a list, tuple,
  dict or set built out of those. Nothing else — no name from the module, no
  attribute, no call inside the call, no arithmetic, no comparison.
- `expect_original` and `expect_mutant` say, in a few words, what each version
  does with that call.

If every input that separates the two needs an argument a literal cannot spell,
answer `distinguishable` and leave `witness` null. That is a real answer and the
right one; an argument outside the rule above cannot be evaluated and is thrown
away.

The witness is run. Both versions of the function are executed on the call you
give, and the two results are compared. A call that does not separate them
establishes nothing, and one whose arguments the function refuses establishes
less than an empty witness.

Reason about the whole function before answering. A guard above the changed text
can make a difference unreachable, and an expression can be rewritten into an
identical one — in both cases the answer is `equivalent`.";

/// Assemble the question about one mutation.
///
/// The change is shown as two texts rather than as a patch. A patch is addressed
/// to a reader with the file in front of them and line numbers to count from, and
/// neither is what is sent: the function arrives on its own, and the two texts are
/// what a search inside it would match.
#[must_use]
pub fn assemble(request: &JudgementRequest) -> Prompt {
    let parts: Vec<String> = vec![
        format!("file: {}", request.file),
        String::new(),
        "```python".to_owned(),
        request.source.trim_end_matches('\n').to_owned(),
        "```".to_owned(),
        String::new(),
        "This change replaces".to_owned(),
        String::new(),
        "```python".to_owned(),
        request.original.trim_end_matches('\n').to_owned(),
        "```".to_owned(),
        String::new(),
        "with".to_owned(),
        String::new(),
        "```python".to_owned(),
        request.replacement.trim_end_matches('\n').to_owned(),
        "```".to_owned(),
        String::new(),
        "Can that change what this function does?".to_owned(),
    ];
    Prompt {
        system: SYSTEM.to_owned(),
        user: parts.join("\n"),
        correction: None,
    }
}

/// The turn that goes back when an answer was not the contract.
///
/// The generating side's wording, unchanged and shared rather than copied: the
/// message says what was wrong with the shape of an answer, which is the same
/// thing to say whatever the answer was about.
#[must_use]
pub fn correction_turn(reason: &str) -> String {
    correction(&[Defect {
        defect: "answer_is_not_the_contract".to_owned(),
        detail: reason.to_owned(),
    }])
}
