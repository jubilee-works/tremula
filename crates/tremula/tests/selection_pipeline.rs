//! Choosing what to mutate out of a real repository's history, and what a generation
//! that chose for itself does about an empty answer.
//!
//! The project here is a git repository with two commits in it, because the first input
//! is a comparison against a revision and there is no honest way to fake one. The
//! language pack is a script: what is under test is the deciding, and the deciding
//! happens before any Python is needed. The provider is a stand-in for the same reason.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::Digest as _;
use tempfile::TempDir;
use tremula_contracts::manifest::{LineRange, Manifest, Span};

use tremula::generate::{
    Attempt, GenerateError, GenerateFailure, GeneratedMutant, GenerationOutcome, GenerationRequest,
    MutantGenerator, Usage,
    command::{self, GenerateArgs},
    selection::{
        conventions::inferred_tests,
        promote::{Candidate, ordered},
    },
};

/// The model every scripted answer names.
const MODEL: &str = "gpt-5.2-2025-12-11";

/// What the pack reports about itself.
const PACK_VERSION: &str = "0.1.0";

/// `ranges.py` as the base commit has it.
const BEFORE: &str = concat!(
    "\"\"\"Ranges.\"\"\"\n",
    "\n",
    "\n",
    "def overlaps(start, end, other_start, other_end):\n",
    "    return start < other_end and other_start < end\n",
);

/// `ranges.py` as the branch has it: a guard added to `overlaps`, and a second
/// function that was not there before.
const AFTER: &str = concat!(
    "\"\"\"Ranges.\"\"\"\n",
    "\n",
    "\n",
    "def overlaps(start, end, other_start, other_end):\n",
    "    if start is None:\n",
    "        return False\n",
    "    return start < other_end and other_start <= end\n",
    "\n",
    "\n",
    "def merge(a, b):\n",
    "    return (min(a[0], b[0]), max(a[1], b[1]))\n",
);

/// The test file the project's convention puts beside `ranges.py`, as the base has it.
const TESTS_BEFORE: &str = "def test_overlaps():\n    assert overlaps(0, 30, 30, 60) is False\n";

/// The same, as the branch has it: a change touches its own tests, and a selection has to
/// say that it left them out rather than that it found nothing.
const TESTS_AFTER: &str = "def test_overlaps():\n    assert overlaps(0, 30, 30, 60) is False\n    assert overlaps(0, 30, 29, 60) is True\n";

/// What every line of the changed file is worth to a test suite: the guard's own
/// `return False` is never reached, and everything else is.
const COVERAGE: &str = concat!(
    "SF:ranges.py\n",
    "DA:4,1\n",
    "DA:5,4\n",
    "DA:6,0\n",
    "DA:7,4\n",
    "DA:10,1\n",
    "DA:11,2\n",
    "end_of_record\n",
);

/// The one expression of `overlaps` a proposal can find.
const CONDITION: &str = "start < other_end and other_start <= end";

/// The one statement of `overlaps` no test reaches.
const UNREACHED: &str = "return False";

/// A project whose history holds the change to select from.
struct Fixture {
    root: TempDir,
}

impl Fixture {
    /// A repository with the base commit and the branch's commit in it.
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let fixture = Self { root };
        let project = fixture.project();
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("ranges.py"), BEFORE).unwrap();
        fs::write(project.join("test_ranges.py"), TESTS_BEFORE).unwrap();
        fixture.git(&["init", "--quiet", "--initial-branch", "main"]);
        fixture.git(&["add", "."]);
        fixture.commit("the base");
        fixture.git(&["branch", "base"]);
        fs::write(project.join("ranges.py"), AFTER).unwrap();
        fs::write(project.join("test_ranges.py"), TESTS_AFTER).unwrap();
        fixture.git(&["add", "."]);
        fixture.commit("the change");
        fixture.write_pack();
        fixture
    }

    fn project(&self) -> PathBuf {
        self.root.path().join("project")
    }

    fn manifest(&self) -> PathBuf {
        self.project().join(command::DEFAULT_MANIFEST)
    }

    fn python(&self) -> PathBuf {
        self.root.path().join("python")
    }

    fn git(&self, arguments: &[&str]) {
        let done = Command::new("git")
            .arg("-C")
            .arg(self.project())
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            done.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&done.stderr)
        );
    }

    fn commit(&self, message: &str) {
        self.git(&[
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "user.name=Fixture",
            "commit",
            "--quiet",
            "--no-gpg-sign",
            "-m",
            message,
        ]);
    }

    /// An interpreter that reports the pack, says where `ranges.py`'s functions are,
    /// and accepts every mutant it is asked about.
    fn write_pack(&self) {
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  *importlib.metadata*)\n    echo '{PACK_VERSION}'\n    exit 0\n    ;;\n  *--capabilities*)\n    echo '{}'\n    exit 0\n    ;;\n  *\"spans --file ranges.py\"*)\n    echo '{}'\n    exit 0\n    ;;\n  *validate*)\n    exit 0\n    ;;\nesac\nexit 2\n",
            capabilities(),
            report(),
        );
        fs::write(self.python(), script).unwrap();
        fs::set_permissions(self.python(), fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Write a coverage document into the project and answer with its path.
    fn coverage(&self, document: &str) -> PathBuf {
        let path = self.project().join("lcov.info");
        fs::write(&path, document).unwrap();
        path
    }

    /// What a person would type to select from this change.
    fn selecting(&self) -> GenerateArgs {
        GenerateArgs {
            file: None,
            functions: Vec::new(),
            diff_base: Some("base".to_owned()),
            coverage: Some(self.coverage(COVERAGE)),
            max_functions: command::DEFAULT_MAX_FUNCTIONS,
            tests: Vec::new(),
            model: MODEL.to_owned(),
            python: Some(self.python()),
            project: self.project(),
            count: 4,
            out: None,
            suppressions: None,
        }
    }

    fn written(&self) -> Manifest {
        serde_json::from_str(&fs::read_to_string(self.manifest()).unwrap()).unwrap()
    }
}

fn capabilities() -> String {
    format!(
        r#"{{"name":"tremula-python","version":"{PACK_VERSION}","contract_version":"0.1","subcommands":["run","collect","validate","spans"],"validate_checks":["compiles_in_file"]}}"#
    )
}

/// Where the two functions of the changed file are, as the pack would report it.
fn report() -> String {
    let digest = format!("{:x}", sha2::Sha256::digest(AFTER.as_bytes()));
    let overlaps = extent("def overlaps", "\n\n\ndef merge");
    let merge = extent("def merge", "");
    format!(
        r#"{{"schema_version":"0.1","file":"ranges.py","file_sha256":"{digest}","functions":[{},{}]}}"#,
        function("overlaps", overlaps),
        function("merge", merge)
    )
}

fn function(name: &str, span: Span) -> String {
    format!(
        r#"{{"qualified_name":"{name}","span":{{"start_byte":{},"end_byte":{}}},"body_span":{{"start_byte":{},"end_byte":{}}}}}"#,
        span.start_byte, span.end_byte, span.start_byte, span.end_byte
    )
}

/// The span of one function of `AFTER`, from where it starts to where the next thing
/// does — or to the end of the file when nothing follows it.
fn extent(from: &str, until: &str) -> Span {
    let start = AFTER.find(from).unwrap();
    let end = if until.is_empty() {
        AFTER.len()
    } else {
        AFTER.find(until).unwrap() + 1
    };
    Span {
        start_byte: u64::try_from(start).unwrap(),
        end_byte: u64::try_from(end).unwrap(),
    }
}

/// A provider that answers every ask the same way, or fails every one of them.
struct Answers {
    mutants: Vec<GeneratedMutant>,
    failure: Option<fn() -> GenerateFailure>,
}

impl Answers {
    /// One mutation of the text named, proposed for every function asked about.
    fn about(original: &str, replacement: &str) -> Self {
        Self {
            mutants: vec![GeneratedMutant {
                file: "ranges.py".to_owned(),
                original: original.to_owned(),
                replacement: replacement.to_owned(),
                description: "an off-by-one at a boundary a test might not pin".to_owned(),
            }],
            failure: None,
        }
    }

    /// A provider that cannot be reached at all.
    fn unreachable() -> Self {
        Self {
            mutants: Vec::new(),
            failure: Some(|| GenerateFailure::Unreachable {
                reason: "connection refused".to_owned(),
            }),
        }
    }

    /// A model that declines to answer, which is about what was asked rather than
    /// about the provider being there.
    fn refusing() -> Self {
        Self {
            mutants: Vec::new(),
            failure: Some(|| GenerateFailure::Refused {
                reason: "this looks like somebody's security code".to_owned(),
            }),
        }
    }
}

impl MutantGenerator for Answers {
    fn generate(&self, _request: &GenerationRequest) -> Result<GenerationOutcome, GenerateError> {
        if let Some(failure) = self.failure {
            return Err(GenerateError::unsent(failure()));
        }
        Ok(GenerationOutcome {
            mutants: self.mutants.clone(),
            model_resolved: MODEL.to_owned(),
            attempts: vec![Attempt {
                usage: Usage {
                    prompt: 100,
                    completion: 50,
                    total: 150,
                },
                finish: Some("stop".to_owned()),
            }],
        })
    }
}

#[test]
fn the_change_a_pull_request_made_becomes_the_functions_it_asks_about() {
    let fixture = Fixture::new();

    let generated = command::compose(
        &fixture.selecting(),
        &Answers::about(CONDITION, "start <= other_end and other_start <= end"),
    )
    .unwrap();

    assert_eq!(command::exit_code(&generated), 0);
    let selection = generated
        .selection
        .as_ref()
        .expect("a selection mode generation records why");
    assert_eq!(selection.diff_base, "base");
    assert_eq!(
        selection.merge_base.as_ref().map(String::len),
        Some(40),
        "the commit the comparison ran from, not the name it was asked for"
    );
    assert_eq!(selection.coverage.as_deref(), Some("lcov.info"));
    assert_eq!(
        selection
            .functions
            .iter()
            .map(|chosen| (chosen.function.as_str(), chosen.candidate_lines))
            .collect::<Vec<(&str, u32)>>(),
        vec![("overlaps", 2), ("merge", 2)],
        "the guard's own unreached line is not a candidate, and the blank lines are not statements"
    );
    let overlaps = &selection.functions[0];
    assert_eq!(
        overlaps.lines,
        LineRange {
            start_line: 4,
            end_line: 7
        }
    );
    assert_eq!(overlaps.inferred_tests, vec!["test_ranges.py".to_owned()]);
    assert_eq!(overlaps.generation.proposed, 1);
    assert_eq!(overlaps.generation.recorded, 1);
    assert!(selection.skipped_over_limit.is_empty());
    assert_eq!(selection.coverage_gaps.len(), 1);
    assert_eq!(
        selection.coverage_gaps[0].ranges,
        vec![LineRange {
            start_line: 6,
            end_line: 6
        }],
        "the changed line no test reached is reported rather than dropped"
    );
    assert!(selection.files_not_in_coverage.is_empty());
    assert_eq!(selection.lines_outside_functions, 0);

    // And the manifest a run would be given carries all of it.
    let written = fixture.written();
    assert_eq!(written.mutants.len(), 1);
    assert_eq!(written.selection.as_ref(), Some(selection));
}

/// The case the whole empty path exists for: a change every line of which is
/// uncovered has nothing to mutate, and the record of that is the most valuable thing
/// such a run produces. A generation that wrote no manifest would destroy it.
#[test]
fn a_change_whose_every_line_is_uncovered_still_writes_its_reasons_and_succeeds() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage(concat!(
        "SF:ranges.py\n",
        "DA:4,1\n",
        "DA:5,0\n",
        "DA:6,0\n",
        "DA:7,0\n",
        "DA:10,0\n",
        "DA:11,0\n",
        "end_of_record\n",
    )));

    let generated = command::compose(&args, &Answers::about(CONDITION, "False")).unwrap();

    assert_eq!(
        command::exit_code(&generated),
        0,
        "nothing to do is not a failure"
    );
    assert!(
        generated.functions.is_empty(),
        "nothing was asked of a model"
    );
    assert_eq!(
        generated.manifest.as_deref(),
        Some(fixture.manifest().as_path())
    );
    let written = fixture.written();
    assert!(written.mutants.is_empty());
    let selection = written
        .selection
        .expect("the reasons are the whole document");
    assert!(selection.functions.is_empty());
    assert_eq!(
        selection.coverage_gaps[0].ranges,
        vec![
            LineRange {
                start_line: 5,
                end_line: 7
            },
            LineRange {
                start_line: 10,
                end_line: 11
            }
        ],
        "the uncovered changed lines, merged where they touch"
    );
}

/// A file the coverage document says nothing at all about is a different fact from a
/// file whose lines it says were never reached, and is reported as its own.
#[test]
fn a_changed_file_the_coverage_document_never_mentions_is_named() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage("SF:elsewhere.py\nDA:1,1\nend_of_record\n"));

    let generated = command::compose(&args, &Answers::about(CONDITION, "False")).unwrap();

    let selection = generated.selection.expect("a selection was still recorded");
    assert_eq!(
        selection.files_not_in_coverage,
        vec!["ranges.py".to_owned()]
    );
    assert!(selection.functions.is_empty());
    assert!(selection.coverage_gaps.is_empty());
}

/// Without coverage every changed line is a candidate, and the lines that belong to no
/// function at all are counted rather than quietly dropped.
#[test]
fn a_selection_made_without_coverage_says_so_and_counts_what_it_could_not_place() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.coverage = None;

    let generated = command::compose(
        &args,
        &Answers::about(CONDITION, "start <= other_end and other_start <= end"),
    )
    .unwrap();

    let selection = generated.selection.as_ref().unwrap();
    assert!(
        selection.coverage.is_none(),
        "a degraded selection says it had no coverage to filter with"
    );
    assert_eq!(
        selection
            .functions
            .iter()
            .map(|chosen| (chosen.function.as_str(), chosen.candidate_lines))
            .collect::<Vec<(&str, u32)>>(),
        vec![("overlaps", 3), ("merge", 2)],
        "every changed line of a function counts when nothing says which ran"
    );
    assert_eq!(
        selection.lines_outside_functions, 2,
        "the two blank lines between the functions belong to neither"
    );
    assert!(selection.coverage_gaps.is_empty());
    let said = tremula::console::render_generation(&generated);
    assert!(said.contains("without coverage"), "{said}");
}

/// Coverage decides which functions are worth asking about, and a model may still
/// propose a mutation anywhere inside one of them. So the last thing before a mutant is
/// recorded is whether any line it replaces was ever reached: a mutation of a line no
/// test runs survives by construction and says nothing about anybody's suite.
#[test]
fn a_mutation_that_lands_where_no_test_ever_went_is_refused() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.max_functions = 1;

    let generated = command::compose(&args, &Answers::about(UNREACHED, "return True")).unwrap();

    assert_eq!(command::exit_code(&generated), 0);
    assert_eq!(generated.recorded, 0);
    let refused = &generated.functions[0].gathered.refused;
    assert_eq!(
        refused.get("mutant_on_uncovered_line"),
        Some(&2),
        "the first ask and the one correction it earns: {refused:?}"
    );
}

/// A model that proposed things and had all of them refused is a normal day, not a
/// broken pipeline. It exits zero — and says so loudly, because a green run that
/// produced no evidence is exactly what a reader must not mistake for a passing one.
#[test]
fn a_selection_no_proposal_of_which_survived_succeeds_and_says_so() {
    let fixture = Fixture::new();

    let generated = command::compose(
        &fixture.selecting(),
        &Answers::about("no such text", "False"),
    )
    .unwrap();

    assert_eq!(command::exit_code(&generated), 0);
    assert_eq!(generated.recorded, 0);
    assert_eq!(
        generated.manifest.as_deref(),
        Some(fixture.manifest().as_path())
    );
    let said = tremula::console::render_generation(&generated);
    assert!(
        said.contains("warning: 2 function(s) were asked about and nothing came of any of them"),
        "{said}"
    );
    let selection = fixture.written().selection.unwrap();
    assert_eq!(selection.functions[0].generation.proposed, 2);
    assert_eq!(selection.functions[0].generation.recorded, 0);
}

/// The one way a selection fails. A key the provider will not accept makes every
/// function's round stop for a reason that has nothing to do with anybody's tests, and
/// a generation that reported success would make a broken credential into a green
/// build forever.
#[test]
fn a_selection_every_function_of_which_could_not_reach_the_provider_fails() {
    let fixture = Fixture::new();

    let generated = command::compose(&fixture.selecting(), &Answers::unreachable()).unwrap();

    assert_eq!(command::exit_code(&generated), 2);
    let said = tremula::console::render_generation(&generated);
    assert!(
        said.contains("could not reach the model provider"),
        "{said}"
    );
}

/// And the case it must not be confused with: a model that answered and declined is
/// the model's own decision about what was asked, which is information rather than a
/// broken pipeline.
#[test]
fn a_selection_whose_functions_the_model_declined_still_succeeds() {
    let fixture = Fixture::new();

    let generated = command::compose(&fixture.selecting(), &Answers::refusing()).unwrap();

    assert_eq!(command::exit_code(&generated), 0);
}

/// A limit that cut something says what it cut. The selected function here is the one
/// that comes first in the file, which is what the order settles when two functions
/// were touched equally.
#[test]
fn the_functions_a_limit_cuts_are_recorded_rather_than_dropped() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.max_functions = 1;

    let generated = command::compose(
        &args,
        &Answers::about(CONDITION, "start <= other_end and other_start <= end"),
    )
    .unwrap();

    let selection = generated.selection.as_ref().unwrap();
    assert_eq!(
        selection
            .functions
            .iter()
            .map(|chosen| chosen.function.as_str())
            .collect::<Vec<&str>>(),
        vec!["overlaps"]
    );
    assert_eq!(
        selection
            .skipped_over_limit
            .iter()
            .map(|over| over.function.as_str())
            .collect::<Vec<&str>>(),
        vec!["merge"]
    );
    let over = &selection.skipped_over_limit[0];
    assert_eq!(over.generation.proposed, 0, "nothing was asked about it");
    assert!(over.inferred_tests.is_empty());
    let said = tremula::console::render_generation(&generated);
    assert!(
        said.contains("selected 1 of 2 functions · 1 capped · 1 test file(s) excluded"),
        "{said}"
    );
}

/// The order a limit cuts in, on its own. Most touched first, because that is where a
/// change is concentrated; then the path and the offset, which are what make two
/// equally touched functions come out the same way on every machine.
#[test]
fn the_order_a_limit_cuts_in_is_settled() {
    let candidates = vec![
        candidate("src/z.py", "last", 1, 40),
        candidate("src/a.py", "second", 2, 90),
        candidate("src/a.py", "first", 2, 10),
        candidate("src/a.py", "most", 9, 500),
    ];

    let sorted = ordered(candidates.clone());

    assert_eq!(
        sorted
            .iter()
            .map(|chosen| chosen.function.as_str())
            .collect::<Vec<&str>>(),
        vec!["most", "first", "second", "last"]
    );
    let mut backwards = candidates;
    backwards.reverse();
    assert_eq!(
        ordered(backwards)
            .iter()
            .map(|chosen| chosen.function.clone())
            .collect::<Vec<String>>(),
        sorted
            .iter()
            .map(|chosen| chosen.function.clone())
            .collect::<Vec<String>>(),
        "the order the candidates arrived in does not decide what a limit keeps"
    );
}

fn candidate(file: &str, function: &str, lines: u32, at: u64) -> Candidate {
    Candidate {
        file: file.to_owned(),
        function: function.to_owned(),
        span: Span {
            start_byte: at,
            end_byte: at + 10,
        },
        lines: LineRange {
            start_line: 1,
            end_line: 2,
        },
        candidate_lines: lines,
    }
}

/// The convention, and its limits. An exact `test_<stem>.py` in a tests directory of a
/// package the file belongs to, or beside the file itself. Nothing else: a project-wide
/// hunt for a similarly named file would send a model somebody else's tests, and every
/// test file found is sent to a provider.
#[test]
fn the_test_file_a_convention_names_is_found_and_nothing_else_is() {
    let project = TempDir::new().unwrap();
    let root = project.path();
    for directory in ["src/pkg", "src/pkg/tests", "tests", "elsewhere"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    for file in [
        "src/pkg/beside.py",
        "src/pkg/test_beside.py",
        "src/pkg/inner.py",
        "src/pkg/tests/test_inner.py",
        "src/pkg/rooted.py",
        "tests/test_rooted.py",
        "src/pkg/lonely.py",
        "src/pkg/nearly.py",
        "elsewhere/test_nearly.py",
        "tests/test_nearlyish.py",
    ] {
        fs::write(root.join(file), "x = 1\n").unwrap();
    }

    assert_eq!(
        inferred_tests(root, "src/pkg/inner.py"),
        vec!["src/pkg/tests/test_inner.py".to_owned()],
        "the tests directory of the file's own package comes first"
    );
    assert_eq!(
        inferred_tests(root, "src/pkg/beside.py"),
        vec!["src/pkg/test_beside.py".to_owned()]
    );
    assert_eq!(
        inferred_tests(root, "src/pkg/rooted.py"),
        vec!["tests/test_rooted.py".to_owned()],
        "the tests directory of a package the file is inside"
    );
    assert!(
        inferred_tests(root, "src/pkg/lonely.py").is_empty(),
        "no test file is an empty list and never a guess"
    );
    assert!(
        inferred_tests(root, "src/pkg/nearly.py").is_empty(),
        "a file of that name somewhere else is not this file's test"
    );
}

/// A directory that happens to be named like a test file is not one.
#[test]
fn a_directory_named_like_a_test_file_is_not_a_test_file() {
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join("tests/test_thing.py")).unwrap();
    fs::write(project.path().join("thing.py"), "x = 1\n").unwrap();

    assert!(inferred_tests(project.path(), "thing.py").is_empty());
}

/// A reference no history has is the one thing a selection cannot go on with, and it
/// says so before anything is asked of a model.
#[test]
fn a_diff_base_this_history_does_not_have_stops_the_generation() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.diff_base = Some("origin/nothing-like-it".to_owned());

    let failure = command::compose(&args, &Answers::about(CONDITION, "False"))
        .expect_err("a comparison against nothing is not a comparison");

    let said = failure.to_string();
    assert!(said.contains("origin/nothing-like-it"), "{said}");
    assert!(!fixture.manifest().exists());
}

/// A coverage document that cannot be read is infrastructure failing, and is not
/// answered by carrying on as though no coverage had been asked for: a run that quietly
/// went degraded would report survivors nobody can interpret.
#[test]
fn a_coverage_document_that_cannot_be_read_stops_the_generation() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage("SF:ranges.py\nDA:this is not a line\n"));

    let failure = command::compose(&args, &Answers::about(CONDITION, "False"))
        .expect_err("a coverage document nobody can read is not coverage");

    assert!(failure.to_string().contains("line 2"), "{failure}");
    assert!(!fixture.manifest().exists());
}

#[test]
fn a_coverage_document_that_is_not_there_stops_the_generation() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.project().join("nothing.info"));

    let failure = command::compose(&args, &Answers::about(CONDITION, "False"))
        .expect_err("a coverage document that is not there is not coverage");

    assert!(failure.to_string().contains("nothing.info"), "{failure}");
}

/// What a selection is measured against is the change itself and not the drift of the
/// branch it came from. The base moving on after this branch left it must not put the
/// base's own commits into this pull request's selection.
#[test]
fn the_comparison_runs_from_where_the_branch_left_the_base() {
    let fixture = Fixture::new();
    // The base gains a commit of its own, touching the same file, after the branch
    // was already taken from it.
    fixture.git(&["checkout", "--quiet", "base"]);
    fs::write(
        fixture.project().join("drift.py"),
        "def unrelated():\n    return 1\n",
    )
    .unwrap();
    fixture.git(&["add", "."]);
    fixture.commit("the base moves on");
    fixture.git(&["checkout", "--quiet", "main"]);
    let mut args = fixture.selecting();
    args.coverage = None;

    let generated = command::compose(
        &args,
        &Answers::about(CONDITION, "start <= other_end and other_start <= end"),
    )
    .unwrap();

    let selection = generated.selection.as_ref().unwrap();
    assert!(
        selection
            .functions
            .iter()
            .all(|chosen| chosen.file == "ranges.py"),
        "the base's own later commit is not this change: {:?}",
        selection.functions
    );
}

/// Naming a function is naming a file, and a selection that also asked about the file
/// somebody named would be two policies at once.
#[test]
fn a_manual_generation_records_no_selection_at_all() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.diff_base = None;
    args.coverage = None;
    args.file = Some("ranges.py".to_owned());
    args.functions = vec!["overlaps".to_owned()];

    let generated = command::compose(
        &args,
        &Answers::about(CONDITION, "start <= other_end and other_start <= end"),
    )
    .unwrap();

    assert!(generated.selection.is_none());
    assert!(fixture.written().selection.is_none());
    assert_eq!(command::exit_code(&generated), 0);
}

/// The manual mode's own promise, unchanged by any of this: a person who named a
/// function and got nothing has a failure, because they asked for something specific.
#[test]
fn a_manual_generation_that_produced_nothing_still_fails() {
    let fixture = Fixture::new();
    let mut args = fixture.selecting();
    args.diff_base = None;
    args.coverage = None;
    args.file = Some("ranges.py".to_owned());
    args.functions = vec!["overlaps".to_owned()];

    let generated = command::compose(&args, &Answers::about("no such text", "False")).unwrap();

    assert_eq!(command::exit_code(&generated), 2);
    assert_eq!(generated.manifest, None, "and writes no manifest");
}

/// A path used to reach the project is not the project's own spelling of it, and a
/// coverage document written under the other one is still about these files.
#[test]
fn a_coverage_document_naming_the_project_absolutely_is_about_these_files() {
    let fixture = Fixture::new();
    let absolute = fs::canonicalize(fixture.project())
        .unwrap()
        .join("ranges.py");
    let document = COVERAGE.replace("SF:ranges.py", &format!("SF:{}", absolute.display()));
    let mut args = fixture.selecting();
    args.coverage = Some(fixture.coverage(&document));

    let generated = command::compose(
        &args,
        &Answers::about(CONDITION, "start <= other_end and other_start <= end"),
    )
    .unwrap();

    let selection = generated.selection.as_ref().unwrap();
    assert_eq!(selection.functions.len(), 2);
    assert!(selection.files_not_in_coverage.is_empty());
}

/// Every path a selection records is the project's own spelling of it, whatever the
/// caller's working directory was.
#[test]
fn the_coverage_a_selection_records_is_the_path_it_was_given() {
    let fixture = Fixture::new();
    let generated =
        command::compose(&fixture.selecting(), &Answers::about(CONDITION, "False")).unwrap();

    let selection = generated.selection.as_ref().unwrap();
    assert_eq!(
        selection.coverage.as_deref(),
        Some("lcov.info"),
        "relative to the project, so the record travels"
    );
    assert!(paths_are_relative(&fixture.project(), selection));
}

fn paths_are_relative(project: &Path, selection: &tremula_contracts::manifest::Selection) -> bool {
    let absolute = project.display().to_string();
    !selection
        .functions
        .iter()
        .any(|chosen| chosen.file.contains(&absolute))
}
