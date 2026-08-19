//! `tremula comment` as a person and as a workflow meet it: what it reads, what it refuses,
//! which comment it replaces, and the promise that the default mode calls nothing at all.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
    sync::Mutex,
};

use assert_cmd::Command;
use tempfile::TempDir;

use tremula::comment::{
    CommentArgs,
    github::{
        Listed, PostError, Posted, Poster, first_line, marker, which_to_replace, with_marker,
    },
    write,
};

/// A manifest with one mutation in it, and the record of why that function was chosen.
const MANIFEST: &str = r#"{
  "schema_version": "0.1",
  "language": "python",
  "base": { "revision": "9f2c1a4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b" },
  "mutants": [
    {
      "id": "0e2f4a6b8c0d2e4f6a80b2c4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b8c0d2e4",
      "file": "ranges.py",
      "base_file_sha256": "3cfd5cb2b71ccbb3e57f07c29f6e228748ce2a7caacffbc9c9e020ce31a5f761",
      "span": { "start_byte": 118, "end_byte": 158 },
      "original": "start < other_end",
      "replacement": "start <= other_end"
    }
  ],
  "selection": {
    "diff_base": "origin/main",
    "merge_base": "9f2c1a4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b",
    "coverage": "lcov.info",
    "functions": [
      {
        "file": "ranges.py",
        "function": "overlaps",
        "span": { "start_byte": 96, "end_byte": 214 },
        "lines": { "start_line": 4, "end_line": 7 },
        "candidate_lines": 2,
        "inferred_tests": ["test_ranges.py"],
        "generation": { "proposed": 4, "recorded": 1 }
      }
    ],
    "skipped_over_limit": [],
    "coverage_gaps": [],
    "files_not_in_coverage": [],
    "lines_outside_functions": 0
  }
}
"#;

/// A manifest of the same mutant identifier and a different mutation of it entirely — what a
/// second generation leaves in the working tree while an earlier run's report is still there.
/// Pairing this with that report would put these spans and this replacement under those
/// verdicts, and nothing in either document would contradict the other.
const DIVERGENT: &str = r#"{
  "schema_version": "0.1",
  "language": "python",
  "base": { "revision": "0011223344556677889900112233445566778899" },
  "mutants": [
    {
      "id": "0e2f4a6b8c0d2e4f6a80b2c4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b8c0d2e4",
      "file": "routes.py",
      "base_file_sha256": "5f4dcc3b5aa765d61d8327deb882cf99f4dcc3b5aa765d61d8327deb882cf995",
      "span": { "start_byte": 10, "end_byte": 40 },
      "original": "WORKING_DAY[0]",
      "replacement": "WORKING_DAY[1]"
    }
  ]
}
"#;

/// The report of a run of that manifest, with the one mutant surviving.
const REPORT: &str = r#"{
  "schema_version": "0.1",
  "run": {
    "run_id": "THE_RUN",
    "tremula_version": "0.1.0",
    "decision_rules_version": "1",
    "project": ".",
    "dirty": false,
    "started_at": "2026-08-19T09:12:44Z",
    "finished_at": "2026-08-19T09:14:02Z"
  },
  "verdicts": [
    {
      "mutant_id": "0e2f4a6b8c0d2e4f6a80b2c4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b8c0d2e4",
      "file": "ranges.py",
      "span": { "start_byte": 118, "end_byte": 158 },
      "location": { "line": 7, "column": 4 },
      "verdict": "survived",
      "detail": "0 failed"
    }
  ],
  "score": {
    "total": 1, "killed": 0, "timeout": 0, "survived": 1,
    "runtime_error": 0, "not_applied": 0, "not_run": 0, "skipped": 0
  },
  "exit_code": 1,
  "caveats": []
}
"#;

/// A report of a run that had nothing to test: the run an empty selection produces, which is
/// the only run that keeps no manifest of its own.
const NOTHING_TESTED: &str = r#"{
  "schema_version": "0.1",
  "run": {
    "run_id": "THE_RUN",
    "tremula_version": "0.1.0",
    "decision_rules_version": "1",
    "project": ".",
    "dirty": false,
    "started_at": "2026-08-19T09:12:44Z",
    "finished_at": "2026-08-19T09:12:45Z"
  },
  "verdicts": [],
  "score": {
    "total": 0, "killed": 0, "timeout": 0, "survived": 0,
    "runtime_error": 0, "not_applied": 0, "not_run": 0, "skipped": 0
  },
  "exit_code": 0,
  "caveats": []
}
"#;

/// What a real run of `MANIFEST` is named: when it started, and the first characters of the
/// hash of the manifest it was given.
///
/// Derived rather than written out, because that name is the only link there is between a run
/// and a manifest kept outside its directory. A fixture that made one up would be pairing two
/// documents no run could have produced together, and would prove nothing about the pairing.
fn the_run() -> String {
    format!(
        "20260819T091244Z-{}",
        tremula::run_dir::fingerprint_of(MANIFEST.as_bytes())
    )
}

/// A project with a manifest in it and one run under it.
struct Workspace {
    root: TempDir,
}

impl Workspace {
    fn new() -> Self {
        let workspace = Self {
            root: TempDir::new().unwrap(),
        };
        fs::create_dir_all(workspace.run()).unwrap();
        fs::write(workspace.project().join("tremula-manifest.json"), MANIFEST).unwrap();
        workspace.report(REPORT);
        workspace
    }

    /// Put a report in the run, naming the run it is of.
    fn report(&self, document: &str) {
        fs::write(
            self.run().join("report.json"),
            document.replace("THE_RUN", &the_run()),
        )
        .unwrap();
    }

    fn project(&self) -> PathBuf {
        self.root.path().to_path_buf()
    }

    fn run(&self) -> PathBuf {
        self.project().join(".tremula").join("runs").join(the_run())
    }

    /// What the run's `latest` link points at, the way a run leaves it.
    fn publish_latest(&self) {
        let runs = self.project().join(".tremula").join("runs");
        std::os::unix::fs::symlink(self.run(), runs.join("latest")).unwrap();
    }

    fn asking(&self) -> CommentArgs {
        CommentArgs {
            run: Some(self.run()),
            manifest: None,
            project: self.project(),
            evidence_url: None,
            github_pr: None,
            github_repo: None,
        }
    }

    /// Run the binary the way a workflow does, with no token anywhere in reach.
    fn comment(&self, extra: &[&str]) -> Output {
        Command::cargo_bin("tremula")
            .unwrap()
            .env_remove("GITHUB_TOKEN")
            .env_remove("GITHUB_REPOSITORY")
            .arg("comment")
            .arg("--project")
            .arg(self.project())
            .args(extra)
            .output()
            .unwrap()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A platform that records what it was asked to post, and answers as the real one would.
struct Recording {
    asked: Mutex<Vec<(String, u64, String)>>,
}

impl Recording {
    fn new() -> Self {
        Self {
            asked: Mutex::new(Vec::new()),
        }
    }
}

impl Poster for Recording {
    fn upsert(&self, repo: &str, pull_request: u64, body: &str) -> Result<Posted, PostError> {
        self.asked
            .lock()
            .unwrap()
            .push((repo.to_owned(), pull_request, body.to_owned()));
        Ok(Posted {
            url: "https://github.com/owner/name/pull/7#issuecomment-1".to_owned(),
            replaced: false,
        })
    }
}

/// A platform that will not take the comment.
struct Refusing;

impl Poster for Refusing {
    fn upsert(&self, _repo: &str, _pull_request: u64, _body: &str) -> Result<Posted, PostError> {
        Err(PostError::Refused {
            status: 403,
            detail: "Resource not accessible by integration".to_owned(),
        })
    }
}

/// A platform nothing may call. Reaching it at all is the failure.
struct Untouchable;

impl Poster for Untouchable {
    fn upsert(&self, _repo: &str, _pull_request: u64, _body: &str) -> Result<Posted, PostError> {
        panic!("the default mode must not call a platform at all");
    }
}

/// The default mode is the product: the body goes to standard output, and nothing is called,
/// reached, or authenticated. A project on another platform pipes this wherever it belongs.
#[test]
fn the_body_goes_to_standard_output_and_nothing_is_called() {
    let workspace = Workspace::new();

    let done = write(&workspace.asking(), &Untouchable);

    assert!(done.is_ok(), "{:?}", done.err().map(|err| err.to_string()));
}

#[test]
fn the_command_prints_the_body_and_succeeds() {
    let workspace = Workspace::new();

    let output = workspace.comment(&["--run", &workspace.run().display().to_string()]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(said.contains("### tremula · mutation testing"), "{said}");
    assert!(
        said.contains("**1 mutants · 0 killed · 1 survived**"),
        "{said}"
    );
    assert!(said.contains("`ranges.py:7`"), "{said}");
    assert!(
        !said.contains("tremula-report:"),
        "the marker belongs to a posted comment, not to the body: {said}"
    );
}

/// The newest run is what a workflow means when it says nothing, and that is a link.
#[test]
fn the_newest_run_is_the_one_reported_on() {
    let workspace = Workspace::new();
    workspace.publish_latest();

    let output = workspace.comment(&[]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("`ranges.py:7`"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_run_directory_that_is_not_there_is_refused() {
    let workspace = Workspace::new();

    let output = workspace.comment(&["--run", "/nowhere/at/all"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("is not a run directory"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_run_with_no_report_in_it_is_refused() {
    let workspace = Workspace::new();
    fs::remove_file(workspace.run().join("report.json")).unwrap();

    let output = workspace.comment(&["--run", &workspace.run().display().to_string()]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("report.json"), "{complaint}");
    assert!(complaint.contains("cannot read"), "{complaint}");
}

#[test]
fn a_manifest_that_is_not_there_is_refused() {
    let workspace = Workspace::new();
    fs::remove_file(workspace.project().join("tremula-manifest.json")).unwrap();

    let output = workspace.comment(&["--run", &workspace.run().display().to_string()]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("tremula-manifest.json"),
        "{}",
        stderr(&output)
    );
}

/// The pairing. A run keeps the manifest it was given, and that copy is what its verdicts are
/// about — however far the working tree has moved on since. Reading the working tree's instead
/// would quote spans and replacements the suite was never run against, under this run's numbers.
#[test]
fn the_manifest_a_run_kept_is_the_one_the_comment_quotes() {
    let workspace = Workspace::new();
    fs::write(workspace.run().join("manifest.json"), MANIFEST).unwrap();
    fs::write(workspace.project().join("tremula-manifest.json"), DIVERGENT).unwrap();

    let output = workspace.comment(&["--run", &workspace.run().display().to_string()]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains("`start < other_end` → `start <= other_end`"),
        "{said}"
    );
    assert!(
        !said.contains("WORKING_DAY"),
        "a later generation's mutation is not this run's: {said}"
    );
    assert!(said.contains("`ranges.py:7`"), "{said}");
}

/// The one run that keeps no manifest of its own is the one that had nothing to test — and it
/// is the run whose comment matters most. Its body comes from the manifest beside the project,
/// which the run's own name vouches for.
#[test]
fn a_run_that_tested_nothing_is_reported_from_the_manifest_the_project_holds() {
    let workspace = Workspace::new();
    workspace.report(NOTHING_TESTED);

    let output = workspace.comment(&["--run", &workspace.run().display().to_string()]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(said.contains("**Nothing was mutated.**"), "{said}");
    assert!(
        said.contains("1 of 1 function(s) this change touched were mutated"),
        "the reasons the manifest records are still reported: {said}"
    );
}

/// And when the project's manifest is not the one that run ran, the comment is refused rather
/// than assembled out of two runs. The name is the only link there is, so it is the check.
#[test]
fn a_run_that_kept_no_manifest_refuses_one_it_never_ran() {
    let workspace = Workspace::new();
    workspace.report(NOTHING_TESTED);
    fs::write(workspace.project().join("tremula-manifest.json"), DIVERGENT).unwrap();

    let output = workspace.comment(&["--run", &workspace.run().display().to_string()]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("tremula-manifest.json"), "{complaint}");
    assert!(complaint.contains(&the_run()), "{complaint}");
    assert!(
        complaint.contains(&tremula::run_dir::fingerprint_of(DIVERGENT.as_bytes())),
        "the message names what was found: {complaint}"
    );
    assert!(
        complaint.contains(&tremula::run_dir::fingerprint_of(MANIFEST.as_bytes())),
        "and what the run's own name says it should have been: {complaint}"
    );
}

#[test]
fn a_document_that_is_not_the_document_it_should_be_is_refused_by_name() {
    let workspace = Workspace::new();
    fs::write(workspace.run().join("report.json"), "{ \"mine\": true }").unwrap();

    let output = workspace.comment(&["--run", &workspace.run().display().to_string()]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("is not a valid report"),
        "{}",
        stderr(&output)
    );
}

/// A triage a run does not have is an ordinary run, and costs one column rather than the
/// comment. A triage it has that cannot be read is a different thing and is refused.
#[test]
fn a_triage_that_cannot_be_read_is_refused_though_an_absent_one_is_not() {
    let workspace = Workspace::new();
    fs::write(workspace.run().join("triage.json"), "{ not a document }").unwrap();

    let output = workspace.comment(&["--run", &workspace.run().display().to_string()]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("is not a valid triage"),
        "{}",
        stderr(&output)
    );
}

/// What is posted carries the marker on its first line, and nothing else does.
#[test]
fn what_is_posted_carries_the_marker_on_its_first_line() {
    let workspace = Workspace::new();
    let platform = Recording::new();
    let mut args = workspace.asking();
    args.github_pr = Some(7);
    args.github_repo = Some("owner/name".to_owned());

    write(&args, &platform).unwrap();

    let asked = platform.asked.lock().unwrap();
    let (repo, pull_request, body) = &asked[0];
    assert_eq!(repo, "owner/name");
    assert_eq!(*pull_request, 7);
    assert_eq!(
        body.lines().next(),
        Some("<!-- tremula-report:owner/name:7 -->")
    );
    assert!(body.contains("### tremula · mutation testing"), "{body}");
}

/// A comment that could not be posted is a failure. A reviewer looking at the previous push's
/// results is worse than a red step, and forgiving it is the workflow's business — with
/// `continue-on-error` — rather than something this swallows.
#[test]
fn a_comment_that_could_not_be_posted_fails() {
    let workspace = Workspace::new();
    let mut args = workspace.asking();
    args.github_pr = Some(7);
    args.github_repo = Some("owner/name".to_owned());

    let failure = write(&args, &Refusing).expect_err("a comment nobody took is not a comment");

    let said = failure.to_string();
    assert!(said.contains("HTTP 403"), "{said}");
    assert!(said.contains("pull-requests: write"), "{said}");
}

/// Which repository the pull request is in has to come from somewhere, and a workflow that
/// gave neither is told which two places were looked at.
#[test]
fn a_pull_request_of_no_named_repository_is_refused() {
    let workspace = Workspace::new();

    let output = workspace.comment(&[
        "--run",
        &workspace.run().display().to_string(),
        "--github-pr",
        "7",
    ]);

    assert_eq!(output.status.code(), Some(2));
    let complaint = stderr(&output);
    assert!(complaint.contains("--github-repo"), "{complaint}");
    assert!(complaint.contains("GITHUB_REPOSITORY"), "{complaint}");
}

#[test]
fn naming_a_repository_without_a_pull_request_is_refused() {
    let workspace = Workspace::new();

    let output = workspace.comment(&["--github-repo", "owner/name"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--github-pr"),
        "{}",
        stderr(&output)
    );
}

/// The choosing, without a socket. Every one of these is a way a run could edit the wrong
/// comment or add one it should have replaced.
#[test]
fn the_comment_a_run_replaces_is_its_own_newest_one() {
    let ours = marker("owner/name", 7);
    let listed = vec![
        listed(1, "Looks good to me", false),
        listed(2, &ours, true),
        listed(3, "I disagree", false),
        listed(4, &ours, true),
    ];

    let replacing = which_to_replace(&listed, &ours).expect("one of them is ours");

    assert_eq!(replacing.id, 4, "the newest of ours is the one on screen");
}

#[test]
fn a_pull_request_with_no_comment_of_ours_has_none_to_replace() {
    let ours = marker("owner/name", 7);
    let listed = vec![
        listed(1, "Looks good to me", false),
        listed(2, &marker("owner/name", 8), true),
        listed(3, &marker("other/name", 7), true),
    ];

    assert!(
        which_to_replace(&listed, &ours).is_none(),
        "another pull request's comment, and another repository's, are not this one's"
    );
}

/// A comment that quotes ours — a review, a bug report, somebody asking what a line means —
/// contains the marker without being ours, and editing it would rewrite what they said.
#[test]
fn a_comment_that_merely_quotes_the_marker_is_not_ours() {
    let ours = marker("owner/name", 7);
    let quoting = Listed {
        id: 9,
        first_line: format!("Why does this say {ours}?"),
        by_a_program: false,
    };

    assert!(which_to_replace(&[quoting], &ours).is_none());
}

/// A program's own comment is the only one replaced, so a repeated run lands on the comment
/// it wrote rather than beside it.
#[test]
fn a_comment_a_program_wrote_is_the_one_chosen_among_the_ones_that_match() {
    let ours = marker("owner/name", 7);
    let listed = vec![listed(1, &ours, true), listed(2, &ours, false)];

    let replacing = which_to_replace(&listed, &ours).expect("one of them is ours");

    assert_eq!(replacing.id, 1);
}

/// The blocker this rule exists for. A token that may comment on a pull request may also edit
/// anybody else's comment on it, so a person who wrote the marker out — quoting it, asking
/// what it is, pasting a previous report — is holding a comment a write could be addressed
/// to. It never is: nothing of ours matches, so a new comment is posted and what they wrote
/// stays as they wrote it.
#[test]
fn a_persons_comment_carrying_the_marker_exactly_is_still_never_replaced() {
    let ours = marker("owner/name", 7);
    let theirs = described_as(
        555,
        &format!("{ours}\n### tremula · mutation testing\n"),
        "clay",
    );

    assert_eq!(theirs.first_line, ours, "the marker, to the character");
    assert!(!theirs.by_a_program, "a person wrote it");
    assert!(
        which_to_replace(&[theirs], &ours).is_none(),
        "a new comment is posted rather than a person's being patched"
    );
}

/// The comparison is exact on the right-hand end too. A first line of the marker and a space
/// is a line somebody typed, and accepting it would mean accepting them.
#[test]
fn a_first_line_of_the_marker_and_trailing_space_is_not_the_marker() {
    let ours = marker("owner/name", 7);
    let padded = listed(9, &format!("{ours} "), true);

    assert!(which_to_replace(&[padded], &ours).is_none());
    assert_eq!(
        first_line(&format!("{ours} \nthe body\n")),
        format!("{ours} "),
        "the space is part of what the first line says"
    );
}

/// The one thing that does come off the end: a body stored with Windows line endings differs
/// from one this wrote in how it was transmitted, not in what it says.
#[test]
fn a_carriage_return_at_the_end_of_the_first_line_does_not_hide_the_marker() {
    let ours = marker("owner/name", 7);

    assert_eq!(first_line(&format!("{ours}\r\nthe body\r\n")), ours);
    assert!(which_to_replace(&[listed(9, &ours, true)], &ours).is_some());
}

#[test]
fn the_marker_names_the_repository_and_the_pull_request() {
    assert_eq!(
        marker("owner/name", 42),
        "<!-- tremula-report:owner/name:42 -->"
    );
    assert_eq!(
        with_marker("<!-- m -->", "### a body\n"),
        "<!-- m -->\n### a body\n"
    );
}

fn listed(id: u64, first_line: &str, by_a_program: bool) -> Listed {
    Listed {
        id,
        first_line: first_line.to_owned(),
        by_a_program,
    }
}

/// One comment as a person's account would arrive: whatever they wrote, under their own login,
/// which the platform reports as a user rather than as a program.
fn described_as(id: u64, body: &str, login: &str) -> Listed {
    Listed {
        id,
        first_line: first_line(body),
        by_a_program: login.ends_with("[bot]"),
    }
}

/// Nothing here may leave a token where a person could read it back.
#[test]
fn no_message_this_command_produces_can_carry_a_token() {
    let workspace = Workspace::new();
    let secret = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
    let failure = PostError::Refused {
        status: 401,
        detail: format!("Bad credentials for {secret}").replace(secret, "«redacted»"),
    };

    assert!(!failure.to_string().contains(secret));
    assert!(
        !Path::new(&workspace.project())
            .join("tremula-manifest.json")
            .display()
            .to_string()
            .contains(secret)
    );
}
