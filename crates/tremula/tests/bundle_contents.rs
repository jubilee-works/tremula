//! What ends up inside a bundle, and whether the bundle is right about it.
//!
//! The index is the only thing a reader has to go on, so the tests here are about it
//! agreeing with the directory it describes: the hashes are the hashes of the files,
//! the paths are paths that exist, the documents are the run's own bytes rather than
//! this version's idea of them, and `START_HERE.md` says something a reader who has
//! never seen this tool can follow.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fs, path::Path};

use serde_json::json;
use sha2::{Digest, Sha256};
use tremula::bundle::{Packaged, package, start_here};
use tremula_contracts::bundle::{Attached, BundleIndex};

mod bundle_fixture;

use bundle_fixture::{FILE, RunFixture, SOURCE, one_survivor, triage};

/// Every path the index names, in one list, so a test can hold all of them to the same
/// promise.
fn attached(index: &BundleIndex) -> Vec<&Attached> {
    let mut named = vec![
        &index.documents.report,
        &index.documents.manifest,
        &index.documents.results,
        &index.documents.baseline,
    ];
    named.extend(index.documents.triage.as_ref());
    named.extend(index.baseline_log.as_ref());
    for attachment in &index.attachments {
        named.extend(attachment.patch.as_ref());
        named.extend(attachment.log.as_ref());
    }
    named
}

/// Every file in a bundle, as a path relative to its root.
fn files(root: &Path, under: &Path, found: &mut Vec<String>) {
    for entry in fs::read_dir(under).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(root, &path, found);
        } else if let Ok(relative) = path.strip_prefix(root) {
            found.push(relative.to_string_lossy().into_owned());
        }
    }
}

/// The whole use of the hashes is that a reader can check them, which means they have
/// to be the hashes of the bytes that were published rather than of anything this
/// process held in memory.
#[test]
fn every_hash_in_the_index_is_the_hash_of_the_file_it_names() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.rewrite(
        "triage.json",
        &triage(&fixture.run_id, "report.json", &fixture.ids[1]),
    );

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("a run with evidence in it produced no bundle");
    };
    let index = fixture.index(&written.path);
    assert_eq!(
        index, written.index,
        "the index on disk is the one reported"
    );
    for named in attached(&index) {
        let path = written.path.join(&named.path);
        let bytes = fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", named.path));
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            named.sha256,
            "{} does not hash to what the index says",
            named.path
        );
    }
}

/// The hashes are worth something only if they cover the bundle. A file that travels
/// without being named in the index is a file no reader can check and no reader was told
/// about, and the index is the only account of the directory there is.
#[test]
fn every_file_in_a_bundle_is_named_in_its_index() {
    let fixture = RunFixture::of(&one_survivor());

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    let index = fixture.index(&written.path);
    let named: Vec<&str> = attached(&index)
        .iter()
        .map(|attached| attached.path.as_str())
        .collect();
    let mut carried = Vec::new();
    files(&written.path, &written.path, &mut carried);
    for file in &carried {
        // The index and the starting document are the two things that describe the rest,
        // so neither can be named by the one of them that does the describing.
        if file == "bundle.json" || file == "START_HERE.md" {
            continue;
        }
        assert!(
            named.contains(&file.as_str()),
            "{file} travels in the bundle and nothing in the index names it: {named:?}"
        );
    }
}

/// The unmutated run's own log belongs to no mutant, so nothing under `attachments` could
/// hold it. A bundle whose only log is that one still carries a log, and both the field a
/// reader checks and the count they see have to say so.
#[test]
fn the_unmutated_run_s_log_is_named_and_counted_on_its_own() {
    let fixture = RunFixture::of(&one_survivor());
    for id in &fixture.ids {
        fs::remove_file(fixture.run_dir().join("logs").join(format!("{id}.txt"))).unwrap();
    }

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    let baseline = written
        .index
        .baseline_log
        .as_ref()
        .expect("the run's own log travelled and the index says nothing about it");
    assert_eq!(baseline.path, "logs/baseline.txt");
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(fs::read(written.path.join(&baseline.path)).unwrap())
        ),
        baseline.sha256
    );
    assert!(
        written.index.exposure.standalone_logs,
        "a log file travels, and the field a reader checks before publishing says it does not"
    );
    for attachment in &written.index.attachments {
        assert!(attachment.log.is_none());
    }
}

/// A document is carried byte for byte. Re-serializing it through this version's own
/// types would publish a document this version agrees with, and the hash a reader
/// checks would be the hash of that instead of of what the run wrote.
#[test]
fn the_run_s_documents_are_carried_byte_for_byte() {
    let fixture = RunFixture::of(&one_survivor());

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    for name in [
        "report.json",
        "manifest.json",
        "results.json",
        "baseline.json",
    ] {
        assert_eq!(
            fs::read(fixture.run_dir().join(name)).unwrap(),
            fs::read(written.path.join(name)).unwrap(),
            "{name} was rewritten on the way into the bundle"
        );
    }
}

/// The working files of a run are not evidence, and freezing them into a bundle would
/// make the execution backend's own database a published contract.
#[test]
fn the_run_s_own_working_files_are_left_out() {
    let fixture = RunFixture::of(&one_survivor());
    for working in [
        "session.sqlite",
        "config.toml",
        "diffs.json",
        "targets.json",
    ] {
        fs::write(fixture.run_dir().join(working), "an internal detail\n").unwrap();
    }

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    for working in [
        "session.sqlite",
        "config.toml",
        "diffs.json",
        "targets.json",
    ] {
        assert!(
            !written.path.join(working).exists(),
            "{working} was published"
        );
    }
    assert!(
        !written.path.join("snapshot").exists(),
        "the project's sources were published"
    );
}

/// A patch in a bundle is a patch git accepted against the bytes the run measured. The
/// check is the point: a bundle whose patches do not apply is a promise it cannot keep.
#[test]
fn a_published_patch_applies_to_the_bytes_the_run_measured() {
    let fixture = RunFixture::of(&one_survivor());

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    assert!(written.unusable.is_empty(), "{:?}", written.unusable);
    for attachment in &written.index.attachments {
        let patch = attachment
            .patch
            .as_ref()
            .expect("every fixture diff applies");
        let elsewhere = fixture.beside(&format!("checkout-{}", attachment.id));
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join(FILE), SOURCE).unwrap();
        let done = std::process::Command::new("git")
            .current_dir(&elsewhere)
            .arg("apply")
            .arg(written.path.join(&patch.path))
            .output()
            .unwrap();
        assert!(
            done.status.success(),
            "{}: {}",
            patch.path,
            String::from_utf8_lossy(&done.stderr)
        );
        let after = fs::read_to_string(elsewhere.join(FILE)).unwrap();
        assert_ne!(after, SOURCE, "the patch changed nothing");
    }
}

/// What a run knew and no document but the report could say: which selectors it was
/// told to collect, and what the unmutated suite came to.
#[test]
fn the_index_carries_the_suite_the_run_measured() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("report.json");
    edited["run"]["tests"] = json!(["tests/test_ranges.py", "tests/test_gaps.py"]);
    edited["run"]["observed_revision"] = json!("9f1c0a3d5e7b2f4a6c8d0e2f4a6b8c0d2e4f6a80");
    fixture.rewrite("report.json", &edited);

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    assert_eq!(
        written.index.suite.tests,
        vec!["tests/test_ranges.py", "tests/test_gaps.py"]
    );
    assert_eq!(written.index.suite.collected, 4);
    assert_eq!(
        written.index.base.revision.as_deref(),
        Some("9f1c0a3d5e7b2f4a6c8d0e2f4a6b8c0d2e4f6a80")
    );
    assert!(written.index.base.reproducible_from_revision);
}

/// A run of a working tree with changes in it started at a revision that is not what it
/// measured, so checking that revision out reproduces something else.
#[test]
fn a_run_of_a_modified_tree_is_not_reproducible_from_its_revision() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("report.json");
    edited["run"]["observed_revision"] = json!("9f1c0a3d5e7b2f4a6c8d0e2f4a6b8c0d2e4f6a80");
    edited["run"]["dirty"] = json!(true);
    fixture.rewrite("report.json", &edited);

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    assert!(written.index.base.dirty);
    assert!(!written.index.base.reproducible_from_revision);
}

/// A project that was not a checkout has no revision, and saying so is not the same as
/// recording nothing.
#[test]
fn a_run_outside_a_checkout_says_there_is_no_revision_to_return_to() {
    let fixture = RunFixture::of(&one_survivor());

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    assert_eq!(written.index.base.revision, None);
    assert!(!written.index.base.reproducible_from_revision);
    let said = fs::read_to_string(written.path.join("START_HERE.md")).unwrap();
    assert!(said.contains("no revision to return to"), "{said}");
    let document = fs::read_to_string(written.path.join("bundle.json")).unwrap();
    assert!(document.contains("\"revision\": null"), "{document}");
}

/// `results.json` holds the execution backend's own output whatever else a bundle
/// leaves out, so the field that says so is not a flag anybody can turn off.
#[test]
fn the_bundle_admits_it_carries_the_backend_s_own_output() {
    let fixture = RunFixture::of(&one_survivor());
    let mut asking = fixture.asking();
    asking.no_logs = true;

    let packaged = package(&asking).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    assert!(written.index.exposure.backend_raw_output);
    assert!(written.index.exposure.absolute_paths);
}

/// The document a reader is meant to open first has to be worth opening: the order to
/// read the documents in, how to reproduce one bug, which suite to run, and the one
/// thing the bundle cannot do.
#[test]
fn the_starting_document_says_how_to_read_the_bundle() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("report.json");
    edited["run"]["tests"] = json!(["tests/test_ranges.py"]);
    edited["run"]["observed_revision"] = json!("9f1c0a3d5e7b2f4a6c8d0e2f4a6b8c0d2e4f6a80");
    fixture.rewrite("report.json", &edited);
    fixture.rewrite(
        "triage.json",
        &triage(&fixture.run_id, "report.json", &fixture.ids[1]),
    );

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    let said = fs::read_to_string(written.path.join("START_HERE.md")).unwrap();
    assert_eq!(said, start_here::render(&written.index));
    assert!(
        said.lines().count() <= 20,
        "a starting document nobody reads is worth nothing: {} lines",
        said.lines().count()
    );
    insta::assert_snapshot!(said);
}

/// The same document for the bundle a reader is most likely to get: no revision, no
/// triage, and the project's own default collection.
#[test]
fn the_starting_document_of_a_bundle_with_nothing_extra_still_says_what_to_do() {
    let fixture = RunFixture::of(&one_survivor());

    let packaged = package(&fixture.asking()).unwrap();

    let Packaged::Written(written) = packaged else {
        panic!("no bundle");
    };
    insta::assert_snapshot!(fs::read_to_string(written.path.join("START_HERE.md")).unwrap());
}
