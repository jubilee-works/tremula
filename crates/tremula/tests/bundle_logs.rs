//! What a log has taken out of it before it travels, and what is left of it.
//!
//! A suite's output is the most useful thing in a bundle and the most revealing: it
//! carries the machine it ran on in every path it prints, and it carries tremula's own
//! protocol in the lines the runner wrote to report itself. Both go, and the order they
//! go in is load-bearing — a log shortened before it is cleaned keeps whatever absolute
//! path happened to straddle the cut, because half a path matches nothing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fmt::Write as _, fs, path::Path};

use tremula::bundle::{
    Packaged,
    logs::{Machine, clean},
    package,
};

mod bundle_fixture;

use bundle_fixture::{RunFixture, one_survivor};

/// How much of each end of a log survives.
const KEPT_EACH_END: usize = 32 * 1024;

/// A machine whose paths are the ones the fixture actually has.
fn machine(project: &Path, run: &Path) -> Machine {
    Machine::around(project, run)
}

fn cleaned(machine: &Machine, raw: &[u8]) -> Vec<u8> {
    clean(machine, raw).bytes
}

/// The whole of it on a log shaped like the real thing: a rootdir, an interpreter under
/// somebody's home directory, a path inside the run, the marker the runner writes, and
/// the temporary report the runner asks pytest for.
#[test]
fn a_suites_output_travels_without_the_machine_it_ran_on() {
    let workspace = tempfile::TempDir::new().unwrap();
    let project = workspace.path().join("sample_project");
    let run = project
        .join(".tremula")
        .join("runs")
        .join("20260810T090000Z-abc123");
    fs::create_dir_all(&run).unwrap();
    let home = std::env::var("HOME").unwrap();
    let raw = format!(
        "============================= test session starts ==============================\n\
         platform darwin -- Python 3.13.1, pytest-8.3.4, pluggy-1.5.0\n\
         rootdir: {project}\n\
         plugins: anyio-4.7.0\n\
         collected 4 items\n\
         \n\
         test_schedule.py ...F                                                    [100%]\n\
         \n\
         =================================== FAILURES ===================================\n\
         _____________________________ test_needs_a_break ______________________________\n\
         \n\
         {project}/test_schedule.py:14: in test_needs_a_break\n\
             assert needs_break(60)\n\
         E   assert False\n\
         \n\
         interpreter: {home}/.venv/bin/python\n\
         bytecode cache: {run}/pycache\n\
         - generated xml file: /somewhere/tremula-junit-a1b2c3/junit.xml -\n\
         =========================== short test summary info ============================\n\
         FAILED test_schedule.py::test_needs_a_break\n\
         TREMULA-RESULT: {{\"collected\": 4, \"failed\": 1, \"passed\": 3, \"pytest_exit\": 1}}\n",
        project = project.display(),
        home = home,
        run = run.display(),
    );

    let said = String::from_utf8(cleaned(&machine(&project, &run), raw.as_bytes())).unwrap();

    assert!(!said.contains(&home), "{said}");
    assert!(!said.contains(&project.display().to_string()), "{said}");
    assert!(!said.contains("TREMULA-RESULT"), "{said}");
    assert!(!said.contains("tremula-junit-"), "{said}");
    insta::assert_snapshot!(said);
}

/// The order the two steps happen in, as a regression. A path that straddles the cut is
/// two halves of a path, and neither half matches anything: shortening first would
/// publish whichever half fell inside the kept region.
#[test]
fn an_absolute_path_across_the_cut_does_not_survive() {
    let workspace = tempfile::TempDir::new().unwrap();
    let project = workspace.path().join("sample_project");
    let run = project.join("runs").join("one");
    fs::create_dir_all(&run).unwrap();
    let spelled = project.display().to_string();
    // Long enough that the path below begins before the cut and ends after it.
    let mut raw = "filler ".repeat(KEPT_EACH_END / 7);
    let straddling = KEPT_EACH_END - spelled.len() / 2;
    raw.truncate(straddling);
    let _ = writeln!(raw, "{spelled}/test_schedule.py:14: in test_needs_a_break");
    raw.push_str(&"trailing ".repeat(KEPT_EACH_END / 9));

    let said = String::from_utf8(cleaned(&machine(&project, &run), raw.as_bytes())).unwrap();

    assert!(said.contains("bytes omitted"), "the log was not shortened");
    assert!(!said.contains(&spelled), "half a path survived the cut");
}

/// A project's own root is spelled one way on the command line and another by anything
/// that resolves it. On this platform `/var` and `/private/var` are the same directory,
/// and a log carries whichever form the process that printed it happened to hold.
#[test]
fn both_the_spelling_and_the_resolved_form_of_a_path_are_taken_out() {
    let workspace = tempfile::TempDir::new().unwrap();
    let project = workspace.path().join("sample_project");
    let run = project.join("runs").join("one");
    fs::create_dir_all(&run).unwrap();
    let resolved = fs::canonicalize(&project).unwrap();
    assert_ne!(
        resolved, project,
        "this test needs a platform where the two forms differ"
    );
    let raw = format!(
        "rootdir: {}\ncachedir: {}/.pytest_cache\n",
        resolved.display(),
        project.display()
    );

    let said = String::from_utf8(cleaned(&machine(&project, &run), raw.as_bytes())).unwrap();

    assert!(!said.contains(&resolved.display().to_string()), "{said}");
    assert!(!said.contains(&project.display().to_string()), "{said}");
}

/// The temporary directory a runner works in is the machine's as much as a home
/// directory is, in both of its spellings.
#[test]
fn the_temporary_directory_is_taken_out_in_both_of_its_spellings() {
    let workspace = tempfile::TempDir::new().unwrap();
    let project = workspace.path().join("sample_project");
    let run = project.join("runs").join("one");
    fs::create_dir_all(&run).unwrap();
    let temporary = std::env::temp_dir();
    let spelled = temporary
        .display()
        .to_string()
        .trim_end_matches('/')
        .to_owned();
    let resolved = fs::canonicalize(&temporary).unwrap();
    let raw = format!(
        "a runner worked in {spelled}/work/one\nand then in {}/work/two\n",
        resolved.display()
    );

    let said = String::from_utf8(cleaned(&machine(&project, &run), raw.as_bytes())).unwrap();

    assert!(!said.contains(&spelled), "{said}");
    assert!(!said.contains(&resolved.display().to_string()), "{said}");
}

/// Paths are replaced and words are not. A name that happens to be somebody's is still
/// a word when it is not part of a path, and a bundle that rewrote every occurrence of
/// it would corrupt the test output it exists to carry.
#[test]
fn a_word_that_is_not_part_of_a_path_is_left_alone() {
    let workspace = tempfile::TempDir::new().unwrap();
    let project = workspace.path().join("clay");
    let run = project.join("runs").join("one");
    fs::create_dir_all(&run).unwrap();
    let raw = format!(
        "assert greet('clay') == 'hello clay'\nrootdir: {}\n",
        project.display()
    );

    let said = String::from_utf8(cleaned(&machine(&project, &run), raw.as_bytes())).unwrap();

    assert!(
        said.contains("assert greet('clay') == 'hello clay'"),
        "{said}"
    );
    assert!(!said.contains(&project.display().to_string()), "{said}");
}

/// The boundary, written down. Sixty-four kilobytes is what a log is allowed to be, and
/// a log of exactly that is not shortened: an off-by-one here would put a notice into
/// every log that came out at the limit.
#[test]
fn a_log_of_exactly_the_limit_is_not_shortened() {
    let workspace = tempfile::TempDir::new().unwrap();
    let project = workspace.path().join("sample_project");
    fs::create_dir_all(&project).unwrap();
    let machine = machine(&project, &project.join("runs").join("one"));

    let exactly = vec![b'x'; KEPT_EACH_END * 2];
    let kept = clean(&machine, &exactly);
    assert!(!kept.truncated);
    assert_eq!(kept.bytes, exactly);

    let one_more = vec![b'x'; KEPT_EACH_END * 2 + 1];
    let shortened = clean(&machine, &one_more);
    assert!(shortened.truncated);
    // One byte over the limit leaves one byte out, and the notice saying so is longer
    // than the byte it replaced. The count is the promise; the length is not.
    assert!(
        String::from_utf8(shortened.bytes.clone())
            .unwrap()
            .contains("… 1 bytes omitted …"),
        "{}",
        String::from_utf8_lossy(&shortened.bytes)
    );
}

/// A count nobody can check is worth nothing, so the number in the notice is the number
/// of bytes that were actually left out of the file that was written.
#[test]
fn the_notice_counts_the_bytes_that_were_left_out() {
    let workspace = tempfile::TempDir::new().unwrap();
    let project = workspace.path().join("sample_project");
    fs::create_dir_all(&project).unwrap();
    let machine = machine(&project, &project.join("runs").join("one"));
    let long = vec![b'x'; KEPT_EACH_END * 3];

    let shortened = clean(&machine, &long);

    let said = String::from_utf8(shortened.bytes.clone()).unwrap();
    let (_, after) = said.split_once("… ").unwrap();
    let (counted, _) = after.split_once(" bytes omitted").unwrap();
    let omitted: usize = counted.parse().unwrap();
    let kept = shortened.bytes.len() - "\n…  bytes omitted …\n".len() - counted.len();
    assert_eq!(omitted, long.len() - kept);
}

/// Test output is whatever a suite printed, which is not always text. A log read as a
/// string would be refused or mangled before any of this could run.
#[test]
fn a_log_that_is_not_text_is_cleaned_all_the_same() {
    let workspace = tempfile::TempDir::new().unwrap();
    let project = workspace.path().join("sample_project");
    let run = project.join("runs").join("one");
    fs::create_dir_all(&run).unwrap();
    let mut raw = b"a test printed this: ".to_vec();
    raw.extend_from_slice(&[0xff, 0xfe, 0x00, 0x80]);
    raw.extend_from_slice(b"\nTREMULA-RESULT: {\"passed\": 1}\n");
    raw.extend_from_slice(format!("rootdir: {}\n", project.display()).as_bytes());

    let said = cleaned(&machine(&project, &run), &raw);

    assert!(
        said.windows(4)
            .any(|window| window == [0xff, 0xfe, 0x00, 0x80]),
        "the bytes a test printed were changed"
    );
    let readable = String::from_utf8_lossy(&said);
    assert!(!readable.contains("TREMULA-RESULT"), "{readable}");
    assert!(
        !readable.contains(&project.display().to_string()),
        "{readable}"
    );
}

/// Cutting inside a character would turn readable output into a decoding error at the
/// one place a reader is most likely to look.
#[test]
fn a_multibyte_character_is_never_cut_in_half() {
    let workspace = tempfile::TempDir::new().unwrap();
    let project = workspace.path().join("sample_project");
    fs::create_dir_all(&project).unwrap();
    let machine = machine(&project, &project.join("runs").join("one"));
    // Every character is three bytes, so no multiple of the cut aligns with one.
    let long = "→".repeat(KEPT_EACH_END);

    let shortened = clean(&machine, long.as_bytes());

    assert!(shortened.truncated);
    let said = String::from_utf8(shortened.bytes)
        .expect("a shortened log has to still be the text it was");
    assert!(said.contains("bytes omitted"), "{said}");
}

/// The logs of a run travel by default, including the one of the run with nothing
/// mutated: a reader comparing a mutant's output against the unmutated suite's needs
/// both, and only one of them is per-mutant.
#[test]
fn the_logs_of_every_mutant_and_of_the_baseline_travel_together() {
    let fixture = RunFixture::of(&one_survivor());

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    assert!(written.index.exposure.standalone_logs);
    assert!(!written.index.logs_truncated);
    for attachment in &written.index.attachments {
        let log = attachment.log.as_ref().expect("every mutant had a log");
        assert!(written.path.join(&log.path).is_file(), "{}", log.path);
    }
    assert!(
        written.path.join("logs").join("baseline.txt").is_file(),
        "the unmutated run's own output was left out"
    );
}

/// `--no-logs` takes the log files out and changes nothing else. It does not make the
/// bundle free of test output: `results.json` carries the backend's own, and the field
/// that says so is not a flag anybody can turn off.
#[test]
fn asking_for_no_logs_leaves_the_backend_s_own_output_where_it_is() {
    let fixture = RunFixture::of(&one_survivor());
    let mut asking = fixture.asking();
    asking.no_logs = true;

    let packaged = package(&asking).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    assert!(!written.index.exposure.standalone_logs);
    assert!(written.index.exposure.backend_raw_output);
    assert!(!written.path.join("logs").exists(), "logs travelled anyway");
    for attachment in &written.index.attachments {
        assert!(attachment.log.is_none());
    }
    let said = fs::read_to_string(written.path.join("START_HERE.md")).unwrap();
    assert!(
        !said.contains("logs/<mutant-id>.txt"),
        "the starting document sent a reader to files that are not there: {said}"
    );
}

/// A log that was shortened is a log a reader has to be told about, once for the
/// bundle: the question they have is whether to trust the logs at all.
#[test]
fn a_bundle_holding_a_shortened_log_says_so() {
    let fixture = RunFixture::of(&one_survivor());
    let chatty = "x".repeat(KEPT_EACH_END * 3);
    fs::write(
        fixture
            .run_dir()
            .join("logs")
            .join(format!("{}.txt", fixture.ids[1])),
        &chatty,
    )
    .unwrap();

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    assert!(written.index.logs_truncated);
    let attached = written.index.attachments[1].log.as_ref().unwrap();
    let said = fs::read_to_string(written.path.join(&attached.path)).unwrap();
    assert!(said.contains("bytes omitted"), "the log was not shortened");
    assert!(said.len() < chatty.len());
}
