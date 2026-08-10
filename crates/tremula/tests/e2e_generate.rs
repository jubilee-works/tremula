//! Generating a manifest and then running it, on a purpose-built project: the real
//! language pack, the real execution backend, and an answer a provider really sent.
//!
//! What the recording proves that a hand-written answer could not: the four
//! mutations in it are what a real model proposed, and three of them are caught by
//! this suite while one is not. A pipeline that dropped a mutant, mislaid a span,
//! or wrote a manifest a run cannot read would fail here rather than in front of
//! somebody.
//!
//! The project, the recording, and what a person would type to generate against
//! them are in `generated_fixture`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod generated_fixture;

use std::{
    fs,
    path::{Path, PathBuf},
};

use tremula::{
    generate::{
        GenerateError, GenerateFailure, GenerationOutcome, GenerationRequest, MutantGenerator,
        command, failures::GenerationFailure, prompt::PROMPT_VERSION,
    },
    validation::canonical_mutant_id,
};
use tremula_contracts::{
    manifest::Manifest,
    report::{Report, Verdict},
};

use generated_fixture::{Fixture, MODEL, OVERLAPPING_CONDITION, Replay, SOMEBODY_ELSES};

#[test]
fn a_recorded_answer_becomes_a_manifest_a_run_consumes() {
    let fixture = Fixture::copy();
    let replay = Replay::of_overlaps();

    let generated = command::compose(&fixture.asking_about("overlaps"), &replay).unwrap();

    assert_eq!(command::exit_code(&generated), 0);
    assert_eq!(generated.recorded, 4, "every recorded mutation survived");
    assert_eq!(
        generated.manifest.as_deref(),
        Some(fixture.manifest().as_path())
    );
    assert_eq!(replay.asked().len(), 1, "nothing needed asking twice");
    assert_eq!(generated.tokens.total, 1118, "the recording's own ledger");

    // The manifest is a document, and what makes it usable is that a run reads it.
    let document = fs::read_to_string(fixture.manifest()).unwrap();
    let manifest: Manifest = serde_json::from_str(&document).unwrap();
    assert_eq!(manifest.mutants.len(), 4);
    let bytes = fs::read(fixture.project().join("ranges.py")).unwrap();
    for mutant in &manifest.mutants {
        let start = usize::try_from(mutant.span.start_byte).unwrap();
        let end = usize::try_from(mutant.span.end_byte).unwrap();
        assert_eq!(&bytes[start..end], mutant.original.as_bytes());
        assert_eq!(
            mutant.id,
            canonical_mutant_id(
                &mutant.file,
                &mutant.span,
                &mutant.base_file_sha256,
                &mutant.replacement
            )
        );
        let generator = mutant.provenance["generator"].as_object().unwrap();
        assert_eq!(generator["model"], serde_json::json!(MODEL));
        assert_eq!(
            generator["prompt_version"],
            serde_json::json!(PROMPT_VERSION)
        );
    }

    let output = fixture.run();

    let said = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{said}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = fixture.report();
    // Three of the four mutations this model proposed are caught by this suite and
    // one is not, which is the whole reason to keep the recording: a run that
    // reported four kills or four survivors would have got something wrong.
    assert_eq!(report.score.total, 4);
    assert_eq!(report.score.killed, 3);
    assert_eq!(report.score.survived, 1);
    assert_eq!(report.score.not_applied, 0);
    // And which mutations they were, not only how many of each. The tally above is
    // the same tally two verdicts swapped between them would produce, and the
    // identifiers are what tie a verdict back to the mutation it is about.
    assert_eq!(
        verdicts_by_replacement(&report, &manifest),
        vec![
            (
                "start < other_end and other_start < other_end",
                Verdict::Killed
            ),
            ("start < other_end and other_start <= end", Verdict::Killed),
            ("start < other_end or other_start < end", Verdict::Killed),
            // The one the suite misses: it pins the touch boundary in the other
            // direction only, so an inclusive `start <= other_end` goes unnoticed.
            (
                "start <= other_end and other_start < end",
                Verdict::Survived
            ),
        ]
    );
}

/// What the run decided about each mutation, named by the replacement it makes.
///
/// The report names mutants by identifier and nothing else, which is what a machine
/// needs and what makes an assertion unreadable. Looking each one up in the manifest
/// the run was given says which mutation it was — and an identifier the manifest
/// does not carry would fail here rather than pass unnoticed.
fn verdicts_by_replacement<'a>(
    report: &'a Report,
    manifest: &'a Manifest,
) -> Vec<(&'a str, Verdict)> {
    let mut named: Vec<(&str, Verdict)> = report
        .verdicts
        .iter()
        .map(|verdict| {
            let mutant = manifest
                .mutants
                .iter()
                .find(|mutant| mutant.id == verdict.mutant_id)
                .expect("every verdict is about a mutant the manifest carries");
            (mutant.replacement.as_str(), verdict.verdict)
        })
        .collect();
    named.sort_unstable_by_key(|(replacement, _)| *replacement);
    named
}

#[test]
fn the_model_is_shown_the_function_its_tests_and_what_not_to_aim_at() {
    let fixture = Fixture::copy();
    let replay = Replay::of_overlaps();

    command::compose(&fixture.asking_about("overlaps"), &replay).unwrap();

    let asked = replay.asked();
    let request = &asked[0];
    assert_eq!(request.file, "ranges.py");
    assert!(
        request.source.starts_with("def overlaps(start: int"),
        "the function's own source, cut from the file at the span the pack reported: {}",
        request.source
    );
    assert!(
        !request.source.contains("def merge"),
        "and nothing else: {}",
        request.source
    );
    assert_eq!(request.tests.len(), 1);
    assert_eq!(request.tests[0].file, "test_ranges.py");
    assert!(
        request.tests[0]
            .source
            .contains("test_ranges_that_only_touch")
    );
    assert_eq!(
        request.excluded,
        vec![r#""""Whether two half-open minute ranges share a minute.""""#],
        "the docstring is named as somewhere not to aim, because a mutation of one changes nothing"
    );
}

#[test]
fn a_generation_no_proposal_survives_writes_nothing_and_fails() {
    // Refused proposals are the ordinary course of asking a model for mutants, and
    // a manifest with none of them in it is still not something to run.
    let fixture = Fixture::copy();

    let generated = command::compose(
        &fixture.asking_about("overlaps"),
        &Replay::about_nothing_that_is_there(),
    )
    .unwrap();

    assert_eq!(generated.recorded, 0);
    assert_eq!(generated.manifest, None);
    assert_eq!(command::exit_code(&generated), 2);
    assert!(!fixture.manifest().exists(), "nothing was written");
    let refused = &generated.functions[0].gathered.refused;
    assert_eq!(refused.get("original_not_found"), Some(&8), "both asks");
}

#[test]
fn a_function_that_stops_leaves_the_others_their_manifest_and_says_so() {
    // The partial-output contract, in one place: a function whose generator gave out
    // fails the generation, and what the functions that finished produced is still
    // written down, because it is real. The exit code and the summary together are
    // what say the output is partial — neither says it alone.
    let fixture = Fixture::copy();
    let mut args = fixture.asking_about("overlaps");
    args.functions.push("merge".to_owned());

    let generated = command::compose(
        &args,
        &AnswersAboutOneFunction {
            replay: Replay::of_overlaps(),
        },
    )
    .expect("one function giving out is not the generation giving out");

    assert_eq!(command::exit_code(&generated), 2);
    assert_eq!(
        generated.manifest.as_deref(),
        Some(fixture.manifest().as_path()),
        "what the function that finished produced is written"
    );
    let manifest: Manifest =
        serde_json::from_str(&fs::read_to_string(fixture.manifest()).unwrap()).unwrap();
    assert_eq!(manifest.mutants.len(), 4);
    assert!(
        manifest
            .mutants
            .iter()
            .all(|mutant| mutant.original == OVERLAPPING_CONDITION),
        "and nothing else: {:?}",
        manifest.mutants
    );
    let said = tremula::console::render_generation(&generated);
    assert!(said.contains("merge: stopped —"), "{said}");
    assert!(said.contains("rate limiting"), "{said}");
    assert!(
        said.contains("partial output: merge did not finish"),
        "{said}"
    );
}

/// A generator that answers about `overlaps` and cannot answer about anything else.
///
/// One function's failure and one function's answer in the same generation, which
/// is the only arrangement that can show what a partial output looks like.
struct AnswersAboutOneFunction {
    replay: Replay,
}

impl MutantGenerator for AnswersAboutOneFunction {
    fn generate(&self, request: &GenerationRequest) -> Result<GenerationOutcome, GenerateError> {
        if !request.source.starts_with("def overlaps") {
            return Err(GenerateError::unsent(GenerateFailure::RateLimited {
                retried: true,
            }));
        }
        self.replay.generate(request)
    }
}

#[test]
fn a_manifest_the_whole_of_which_is_not_valid_is_not_written() {
    // Every mutant was checked one at a time, and there is one thing no per-mutant
    // answer can establish: that the manifest holds together. Naming a function
    // twice asks the same question twice and gets the same mutations back, which is
    // one identifier twice — and a manifest that carries one twice is refused whole
    // by the run it was written for, so it is refused here instead.
    let fixture = Fixture::copy();
    let mut args = fixture.asking_about("overlaps");
    args.functions.push("overlaps".to_owned());

    let failure = command::compose(&args, &Replay::of_overlaps())
        .expect_err("a manifest with one identifier twice is not one to write");

    assert!(
        matches!(failure, GenerationFailure::Invalid(_)),
        "{failure}"
    );
    assert!(failure.to_string().contains("more than once"), "{failure}");
    assert!(!fixture.manifest().exists(), "nothing was written");
}

#[test]
fn a_generation_never_writes_over_a_manifest_that_is_already_there() {
    let fixture = Fixture::copy();
    fs::write(fixture.manifest(), SOMEBODY_ELSES).unwrap();

    let failure = command::compose(&fixture.asking_about("overlaps"), &Replay::of_overlaps())
        .expect_err("an existing manifest is never written over");

    assert!(
        matches!(failure, GenerationFailure::ManifestExists { .. }),
        "{failure}"
    );
    assert!(failure.to_string().contains("--out"), "{failure}");
    assert_eq!(
        fs::read_to_string(fixture.manifest()).unwrap(),
        SOMEBODY_ELSES,
        "and what was there is untouched"
    );
}

#[test]
fn a_manifest_that_appears_during_a_generation_is_not_written_over_either() {
    // The check at the start cannot be the whole of the promise: a generation waits
    // on a model, and the file it is going to write can be created in that time.
    let fixture = Fixture::copy();
    let generator = WritesTheManifestFirst {
        replay: Replay::of_overlaps(),
        manifest: fixture.manifest(),
    };

    let failure = command::compose(&fixture.asking_about("overlaps"), &generator)
        .expect_err("a manifest that appeared in the meantime is somebody's file too");

    assert!(
        matches!(failure, GenerationFailure::ManifestExists { .. }),
        "{failure}"
    );
    assert_eq!(
        fs::read_to_string(fixture.manifest()).unwrap(),
        SOMEBODY_ELSES,
        "and what appeared is untouched"
    );
    assert_eq!(
        half_written_beside(&fixture.manifest()),
        Vec::<String>::new(),
        "and nothing half-written is left where the manifest was going"
    );
}

/// A generator that puts a file where the manifest is going to go.
///
/// Which is what a person, or a second generation, does while this one is waiting
/// on a model — and the reason the manifest is committed by linking it to a name
/// nothing holds rather than by renaming a file over whatever is there.
struct WritesTheManifestFirst {
    replay: Replay,
    manifest: PathBuf,
}

impl MutantGenerator for WritesTheManifestFirst {
    fn generate(&self, request: &GenerationRequest) -> Result<GenerationOutcome, GenerateError> {
        let answer = self.replay.generate(request);
        fs::write(&self.manifest, SOMEBODY_ELSES).unwrap();
        answer
    }
}

/// Everything in the manifest's directory that is a staging file rather than a
/// document somebody asked for.
fn half_written_beside(manifest: &Path) -> Vec<String> {
    let mut left = Vec::new();
    for entry in fs::read_dir(manifest.parent().unwrap()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if name.contains(".part") {
            left.push(name);
        }
    }
    left
}

#[test]
fn a_generation_goes_ahead_while_a_run_holds_the_project() {
    // A generation reads and never patches a source, so it takes no lock. What a
    // concurrent run could do is change the file underneath it, and that is caught
    // by reading the hash again before anything is written — and by the run that
    // tried to use such a manifest, since a mutant's identifier is derived from
    // the hash of the file it was generated against.
    let fixture = Fixture::copy();
    let directory = fixture.project().join(".tremula");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("lock"),
        format!(
            r#"{{"run_id":"20260810T000000Z-abcdef","pid":{}}}"#,
            std::process::id()
        ),
    )
    .unwrap();

    let generated = command::compose(&fixture.asking_about("overlaps"), &Replay::of_overlaps())
        .expect("a generation does not need the project lock");

    assert_eq!(generated.recorded, 4);
}

#[test]
fn a_file_that_moves_under_a_generation_is_never_written_down() {
    let fixture = Fixture::copy();
    let replay = MovesTheFile {
        replay: Replay::of_overlaps(),
        file: fixture.project().join("ranges.py"),
    };

    let failure = command::compose(&fixture.asking_about("overlaps"), &replay)
        .expect_err("offsets measured against bytes that have gone are not written down");

    assert!(
        matches!(failure, GenerationFailure::SourcesChanged { .. }),
        "{failure}"
    );
    assert!(!fixture.manifest().exists(), "nothing was written");
}

/// A generator that edits the file between being asked and answering.
///
/// Which is what a run of the same project would do, and the reason a generation
/// reads the hash again before it writes anything.
struct MovesTheFile {
    replay: Replay,
    file: PathBuf,
}

impl MutantGenerator for MovesTheFile {
    fn generate(&self, request: &GenerationRequest) -> Result<GenerationOutcome, GenerateError> {
        let answer = self.replay.generate(request);
        let source = fs::read_to_string(&self.file).unwrap();
        fs::write(&self.file, format!("{source}\n\n# a later thought\n")).unwrap();
        answer
    }
}

/// A candidate somebody has dismissed is never recorded again.
///
/// The recording proposes four mutations of `overlaps`; one of them is dismissed
/// before the generation runs, and the manifest comes back with three. The decision
/// is written by hand rather than by `dismiss` because there is no run to look a
/// mutant up in yet — which is itself the point of keying a decision by the mutation
/// instead of by an identifier no manifest has produced.
#[test]
fn a_candidate_a_person_dismissed_is_not_recorded_again() {
    let fixture = Fixture::copy();
    let replay = Replay::of_overlaps();
    let dismissed = "start <= other_end and other_start < end";
    fs::write(
        fixture
            .project()
            .join(tremula::suppressions::DEFAULT_SUPPRESSIONS),
        serde_json::json!({
            "schema_version": "0.1",
            "suppressions": [{
                "file": "ranges.py",
                "original": OVERLAPPING_CONDITION,
                "replacement": dismissed,
                "reason": "not_useful",
                "dismissed_at": "2026-08-10T09:12:00Z",
            }],
        })
        .to_string(),
    )
    .unwrap();

    let generated = command::compose(&fixture.asking_about("overlaps"), &replay).unwrap();

    assert_eq!(
        generated.recorded, 3,
        "the dismissed one of the four is gone"
    );
    assert_eq!(generated.functions[0].gathered.suppressed, 1);
    assert_eq!(
        generated.functions[0].gathered.proposed, 4,
        "what the model proposed is still what it proposed"
    );
    let document = fs::read_to_string(fixture.manifest()).unwrap();
    assert!(!document.contains(dismissed), "{document}");
    let manifest: Manifest = serde_json::from_str(&document).unwrap();
    assert_eq!(manifest.mutants.len(), 3);
}

/// A record of decisions that cannot be read stops a generation before it pays for
/// anything.
#[test]
fn a_record_of_decisions_that_cannot_be_read_stops_a_generation_before_it_asks() {
    let fixture = Fixture::copy();
    let replay = Replay::of_overlaps();
    fs::write(
        fixture
            .project()
            .join(tremula::suppressions::DEFAULT_SUPPRESSIONS),
        "{ not a document }",
    )
    .unwrap();

    let failure = command::compose(&fixture.asking_about("overlaps"), &replay).unwrap_err();

    assert!(
        matches!(failure, GenerationFailure::Suppressions(_)),
        "{failure}"
    );
    assert!(replay.asked().is_empty(), "nothing was asked of the model");
    assert!(!fixture.manifest().exists());
}

/// One real generation about one function, skipped unless the environment names
/// both a key and a model.
///
/// Not part of any gate: nothing here can make a network call answer. It exists to
/// be run deliberately by somebody who has a key, and it costs up to two calls —
/// the ask, and the one correction a round spends when a proposal is refused.
#[test]
fn a_real_provider_generates_a_manifest_for_one_function() {
    let (Ok(_), Ok(model)) = (
        std::env::var(tremula::generate::openai::KEY_VARIABLE),
        std::env::var("TREMULA_LIVE_MODEL"),
    ) else {
        eprintln!(
            "skipped: set {} and TREMULA_LIVE_MODEL to generate against a real provider",
            tremula::generate::openai::KEY_VARIABLE
        );
        return;
    };
    let fixture = Fixture::copy();
    let mut args = fixture.asking_about("overlaps");
    args.model = model;

    let generated = command::compose(
        &args,
        &tremula::generate::openai::OpenAiGenerator::new(&args.model),
    )
    .expect("a real provider answers about one function");

    assert!(generated.tokens.total > 0);
    assert!(
        generated.recorded > 0,
        "at least one proposal survived every check"
    );
}
