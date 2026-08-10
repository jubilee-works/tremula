//! How often the judging is wrong, measured against mutations whose equivalence is
//! already known.
//!
//! `tests/fixtures/labelled-mutations/` holds six frozen modules and every mutation
//! of them that a model proposed and a person then labelled one at a time. Sixteen of
//! the thirty-eight change nothing the surrounding program can reach; twenty-two do.
//! This target runs the whole of triage over them and reports the confusion matrix.
//!
//! # What this pins
//!
//! The matrix itself, cell by cell, exactly as it was measured — both labels against
//! every classification. Any drift in any direction fails, including a cell that gets
//! better: a number nobody has to look at when it moves is not a measurement, and the
//! honest way for one of these to change is for somebody to change it on purpose and
//! say here what it now says.
//!
//! There is no threshold in this file that the measurement does not already meet. A
//! gate that fails from the day it is written is a gate that gets switched off, and a
//! number this project would like to reach is a sentence in a document, not an
//! assertion.
//!
//! Three cells are disagreements between the labels and the classifications, and each
//! is named by the mutation it is about rather than only counted, so that a different
//! mutation drifting into one of them fails while the count still holds. The reasons
//! are on `DISTINGUISHED_THOUGH_EQUIVALENT` and `SUSPECTED_THOUGH_NOT_EQUIVALENT`
//! below: two are a limit probing has by construction, and one is a real miss, kept on
//! the record rather than rounded off.
//!
//! Recall is not pinned as a bar to clear, only as the number it is. A survivor nobody
//! could establish anything about costs a person a look, which is what they were going
//! to do anyway.
//!
//! # Replayed, and how it was recorded
//!
//! The judgements are on disk, so this target makes no network call and says the same
//! thing every time. Everything else is real: the real pack, and every witness really
//! executed against the two real versions of a frozen file.
//!
//! To record them again — which is the only thing here that costs money — set
//! `TREMULA_JUDGE_RECORD=1` with a key in the environment. The ceilings the measurement
//! was taken under are enforced in `labelled_mutations`, before a call is sent rather
//! than after it is paid for.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod labelled_mutations;

use std::{collections::BTreeMap, fs};

use labelled_mutations::{
    Golden, Labelled, MAX_CALLS, MAX_TOTAL_TOKENS, RECORD_VARIABLE, Recorded, labels, recordings,
};
use tremula::{
    generate::{judge::JudgementRequest, openai::KEY_VARIABLE},
    triage::assess,
};
use tremula_contracts::triage::{Classification, Triage, TriageEntry};

/// One labelled mutation, named the way a person can find it again: the file, the text
/// replaced, the text put there. The same three fields a dismissal is keyed by, and
/// none of them a hash that changes when the file does.
type Identity = (&'static str, &'static str, &'static str);

/// The measurement, cell by cell: the label on the left, the classification triage
/// reached on the right, and how many mutations met that way.
///
/// Hand-labelled against `gpt-5.2-2025-12-11`'s answers as they are recorded under the
/// fixture's `judgements/`. Pinned exactly, so that any movement in any direction fails
/// and gets read by somebody.
const MEASURED: [(bool, &str, usize); 6] = [
    (false, "distinguished_at_function_level", 8),
    (false, "suspected_equivalent", 1),
    (false, "undecided", 13),
    (true, "distinguished_at_function_level", 2),
    (true, "suspected_equivalent", 8),
    (true, "undecided", 6),
];

/// Equivalent mutations that probing separates anyway, and why that is accepted.
///
/// Both are in `exit_class_of`, whose only caller builds the mapping it passes and
/// never leaves either key out — so `marker["k"]` and `marker.get("k")` cannot differ
/// anywhere in the program. Called on its own, which is exactly what a probe does, the
/// function is handed a mapping that never passed that caller, and the two versions
/// differ on the first input that omits the key.
///
/// A limit of the method rather than a wrong answer: the classification is named
/// `distinguished_at_function_level` because a distinction at the level of the function
/// is the whole of what it claims, and here it is that and no more. The fixture
/// pre-registers the pair as `separable_at_function_level`, so this measurement is not
/// also a measurement of a limit already known.
const DISTINGUISHED_THOUGH_EQUIVALENT: [Identity; 2] = [
    (
        "marker.py",
        "marker[\"pytest_exit\"]",
        "marker.get(\"pytest_exit\")",
    ),
    (
        "marker.py",
        "marker[\"timed_out\"]",
        "marker.get(\"timed_out\")",
    ),
];

/// The one mutation labelled not equivalent that the judging called
/// `suspected_equivalent`, and why that is accepted as a known miss.
///
/// The interval rewrite in `overlaps`. The two forms agree on every pair of ranges in
/// which both are non-empty; the only inputs that separate them have one range of zero
/// length, such as `overlaps(5, 5, 5, 10)`. The labels take the strict reading —
/// equivalent only when *no* input the program can reach separates the two — and
/// nothing between `overlaps` and its callers rules a zero-length range out, so it is
/// labelled not equivalent. The model judged it equivalent. That is a miss in the
/// direction that matters, and it is named here rather than smoothed away.
///
/// Accepted, because of what the classification is: `suspected_equivalent` records that
/// a model said a mutation changes nothing and that *nothing ran to check it*. It
/// retires no survivor and dismisses nothing — it is a suggestion for a person to
/// verify, advisory by design — so a miss there costs a reader the same look they were
/// going to take. What would not be acceptable is a second one arriving unnoticed,
/// which is what naming this one prevents.
const SUSPECTED_THOUGH_NOT_EQUIVALENT: [Identity; 1] = [(
    "ranges.py",
    "return start < other_end and other_start < end",
    "return (start <= other_start < end) or (other_start <= start < other_end)",
)];

/// Which mutations met each label with each classification.
///
/// The mutations themselves rather than a count, because how many are in a cell is one
/// assertion and which ones they are is another.
fn matrix<'a>(
    judged: &Triage,
    mutations: &'a [Labelled],
) -> BTreeMap<(bool, &'static str), Vec<&'a Labelled>> {
    let mut found: BTreeMap<(bool, &'static str), Vec<&'a Labelled>> = BTreeMap::new();
    for mutation in mutations {
        let entry = of(judged, mutation);
        found
            .entry((mutation.equivalent, named(entry.classification)))
            .or_default()
            .push(mutation);
    }
    found
}

/// One cell's contents, or nothing if no mutation landed there.
fn cell<'a>(
    found: &'a BTreeMap<(bool, &'static str), Vec<&'a Labelled>>,
    equivalent: bool,
    classification: &'static str,
) -> &'a [&'a Labelled] {
    found
        .get(&(equivalent, classification))
        .map_or(&[], Vec::as_slice)
}

/// One mutation as a line a person can find again in `labels.json`.
fn describe(mutation: &Labelled) -> String {
    format!(
        "{}/{}: {} → {} [{}]",
        mutation.file, mutation.function, mutation.original, mutation.replacement, mutation.label
    )
}

/// Whether a mutation is the one an identity names.
fn is(mutation: &Labelled, named: Identity) -> bool {
    mutation.file == named.0 && mutation.original == named.1 && mutation.replacement == named.2
}

/// Assert that a cell holds exactly the mutations named for it.
///
/// By identity, on top of the count the matrix already pins, so that a new disagreement
/// trading places with an accepted one fails instead of cancelling out.
fn holds_exactly(found: &[&Labelled], named: &[Identity], what: &str) {
    let strangers: Vec<String> = found
        .iter()
        .filter(|mutation| !named.iter().any(|one| is(mutation, *one)))
        .map(|mutation| describe(mutation))
        .collect();
    assert!(
        strangers.is_empty(),
        "{what}, and this project has not accepted that: {strangers:#?}"
    );
    for one in named {
        assert!(
            found.iter().any(|mutation| is(mutation, *one)),
            "`{} → {}` is an accepted disagreement — {what} — and it is no longer one. \
             Say what changed rather than deleting the entry.",
            one.1,
            one.2
        );
    }
}

/// The entry a triage reached about one labelled mutation.
fn of<'a>(judged: &'a Triage, mutation: &Labelled) -> &'a TriageEntry {
    judged
        .entries
        .iter()
        .find(|entry| {
            entry.file == mutation.file
                && entry.original == mutation.original
                && entry.replacement == mutation.replacement
        })
        .unwrap_or_else(|| {
            panic!(
                "the triage said nothing about {} → {}",
                mutation.original, mutation.replacement
            )
        })
}

/// A classification as the document spells it, which is how the matrix is written down.
fn named(classification: Classification) -> &'static str {
    match classification {
        Classification::DistinguishedAtFunctionLevel => "distinguished_at_function_level",
        Classification::SuspectedEquivalent => "suspected_equivalent",
        Classification::Undecided => "undecided",
        Classification::Unknown => "unknown",
    }
}

#[test]
fn the_fixture_still_holds_what_the_measurement_was_taken_over() {
    let mutations = labels();
    assert_eq!(mutations.len(), 38);
    assert_eq!(mutations.iter().filter(|one| one.equivalent).count(), 16);
    assert_eq!(mutations.iter().filter(|one| !one.equivalent).count(), 22);
    let pinned: usize = MEASURED.iter().map(|(_, _, count)| count).sum();
    assert_eq!(
        pinned,
        mutations.len(),
        "the pinned matrix accounts for every labelled mutation and no others"
    );

    // The mutations named as separable anyway are the ones the fixture registered, so
    // neither list can be edited into agreement with the other on its own.
    let separable: Vec<&Labelled> = mutations
        .iter()
        .filter(|one| one.separable_at_function_level)
        .collect();
    holds_exactly(
        &separable,
        &DISTINGUISHED_THOUGH_EQUIVALENT,
        "the fixture registers it as separable when the function is called directly",
    );
    assert!(
        separable.iter().all(|one| one.equivalent),
        "a mutation registered as separable-anyway is one of the equivalent ones"
    );

    // And the accepted miss is a mutation the fixture really does label not equivalent,
    // so that acceptance cannot quietly become the acceptance of something else.
    for one in &SUSPECTED_THOUGH_NOT_EQUIVALENT {
        let found = mutations
            .iter()
            .find(|mutation| is(mutation, *one))
            .unwrap_or_else(|| panic!("`{} → {}` is not in the fixture", one.1, one.2));
        assert!(
            !found.equivalent,
            "`{} → {}` is accepted as a miss against a not-equivalent label",
            one.1, one.2
        );
    }
}

#[test]
fn the_confusion_matrix_is_the_measured_one_and_the_misses_are_the_named_ones() {
    let mutations = labels();
    let judge = if std::env::var(RECORD_VARIABLE).is_ok() {
        assert!(
            std::env::var(KEY_VARIABLE).is_ok(),
            "recording needs {KEY_VARIABLE} in the environment"
        );
        Recorded::recording(recordings())
    } else {
        Recorded::replaying(recordings())
    };
    let golden = Golden::of(&mutations);

    let judged = assess(&golden.asking(), &judge).unwrap();

    let found = matrix(&judged, &mutations);
    println!("\nconfusion matrix — label against classification\n");
    for ((equivalent, classification), listed) in &found {
        let label = if *equivalent {
            "equivalent"
        } else {
            "not equivalent"
        };
        println!("{label} × {classification}: {}", listed.len());
        for one in listed {
            println!("    {}", describe(one));
        }
    }
    println!("what each answer came to");
    let mut said: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &judged.entries {
        let key = match (entry.classification, entry.undecided) {
            (Classification::Undecided, Some(why)) => {
                format!("undecided: {}", tremula::triage::classify::reason(why))
            }
            (classification, _) => named(classification).to_owned(),
        };
        *said.entry(key).or_default() += 1;
    }
    for (what, count) in &said {
        println!("    {what}: {count}");
    }
    let (calls, tokens) = judge.spent();
    println!(
        "\ncalls: {calls}/{MAX_CALLS} · tokens: {tokens}/{MAX_TOTAL_TOKENS} · document spend: \
         {} calls, {} tokens\n",
        judged.spend.calls, judged.spend.total_tokens
    );

    // The matrix, pinned whole rather than a cell at a time, so that a classification
    // landing where none was measured — `unknown`, say — fails for want of a row instead
    // of passing for want of an assertion about it.
    let counted: BTreeMap<(bool, &str), usize> = found
        .iter()
        .map(|(where_it_met, listed)| (*where_it_met, listed.len()))
        .collect();
    let pinned: BTreeMap<(bool, &str), usize> = MEASURED
        .iter()
        .map(|(equivalent, classification, count)| ((*equivalent, *classification), *count))
        .collect();
    assert_eq!(
        counted, pinned,
        "the confusion matrix is not the one on the record. Every cell of it is a \
         measurement of this tool, so a move in any direction — a better number \
         included — changes what the project claims: read the matrix printed above, and \
         pin what it now says."
    );

    // The two equivalent mutations a direct call separates anyway: accepted, because a
    // caller's guard is not something probing can see. The reason is on the constant.
    holds_exactly(
        cell(&found, true, "distinguished_at_function_level"),
        &DISTINGUISHED_THOUGH_EQUIVALENT,
        "it is equivalent and was distinguished at function level",
    );

    // The one not-equivalent mutation called suspected_equivalent: accepted as a known
    // miss, because that classification is a model's suggestion for a person to verify
    // and retires nothing by itself. The reason is on the constant.
    holds_exactly(
        cell(&found, false, "suspected_equivalent"),
        &SUSPECTED_THOUGH_NOT_EQUIVALENT,
        "it changes what the program does and was called suspected_equivalent",
    );
}

/// The judgements on disk are what the measurement stands on, so they are held to being
/// about the fixture that is there now.
#[test]
fn every_labelled_mutation_has_a_recorded_judgement() {
    if std::env::var(RECORD_VARIABLE).is_ok() {
        eprintln!("skipped: this run is recording them");
        return;
    }
    let judge = Recorded::replaying(recordings());
    let mut missing = Vec::new();
    for mutation in labels() {
        let request = JudgementRequest {
            file: mutation.file.clone(),
            source: String::new(),
            original: mutation.original.clone(),
            replacement: mutation.replacement.clone(),
        };
        let path = judge.kept_at(&request);
        if !path.is_file() {
            missing.push(format!("{} → {}", mutation.original, mutation.replacement));
        }
    }
    assert!(missing.is_empty(), "no recorded judgement for {missing:#?}");
    let kept = fs::read_dir(recordings()).unwrap().count();
    assert_eq!(
        kept, 38,
        "the recordings are exactly the fixture's mutations, with none left over from a \
         fixture that has since changed"
    );
}

/// A claim of equivalence is the one answer nothing checks, so what the model said is
/// recorded and can be read back.
#[test]
fn what_the_model_claimed_is_recorded_beside_every_classification() {
    if std::env::var(RECORD_VARIABLE).is_ok() {
        eprintln!("skipped: this run is recording them");
        return;
    }
    let mutations = labels();
    let golden = Golden::of(&mutations);
    let judge = Recorded::replaying(recordings());

    let judged = assess(&golden.asking(), &judge).unwrap();

    for mutation in &mutations {
        let entry = of(&judged, mutation);
        assert!(
            entry.claim.is_some(),
            "a judgement was obtained about `{} → {}`, so its claim is on the record",
            mutation.original,
            mutation.replacement
        );
        if entry.classification == Classification::SuspectedEquivalent {
            assert_eq!(
                entry.claim,
                Some(tremula_contracts::triage::Claim::Equivalent)
            );
            assert!(
                entry.probe.is_none(),
                "nothing was run, so nothing is recorded"
            );
        }
    }
    assert!(
        judged
            .entries
            .iter()
            .filter(|entry| entry.classification == Classification::DistinguishedAtFunctionLevel)
            .all(|entry| entry.probe.is_some()),
        "the one classification with evidence carries it"
    );
}
