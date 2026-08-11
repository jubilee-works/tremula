//! What `tremula bundle` refuses, and what it leaves behind when it does.
//!
//! Every case here is one of the two promises the command makes about failing. The
//! first is that a bundle is never a mixture: documents of two runs, a triage about
//! another report, or a manifest and a report covering different mutants are all one
//! run's evidence turning out to be two, and a reader of the finished directory would
//! have no way to notice. The second is that a failure leaves nothing: no bundle at
//! the output path, and no staging directory beside it either.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;

use serde_json::json;
use tremula::bundle::{exit_status, package};

mod bundle_fixture;

use bundle_fixture::{FILE, PROJECT, RunFixture, one_survivor, report, triage};

/// The default `--run` is the `latest` link, and a link's own name is not a run
/// identifier. Comparing it against the documents would make every default
/// invocation fail with a complaint about the wrong thing.
#[test]
fn packaging_with_no_run_named_follows_the_latest_link() {
    let fixture = RunFixture::of(&one_survivor());

    let code = exit_status(&fixture.asking());

    assert_eq!(code, 0);
    let out = fixture.default_out();
    assert!(out.join("bundle.json").is_file(), "{}", out.display());
    assert_eq!(fixture.index(&out).run_id, fixture.run_id);
}

/// A bundle is a thing that gets copied and sent, so the one already at a path may be
/// the copy somebody was given.
#[test]
fn a_path_that_already_holds_something_is_never_written_over() {
    let fixture = RunFixture::of(&one_survivor());
    let taken = fixture.beside("taken");
    fs::create_dir(&taken).unwrap();
    fs::write(taken.join("something-of-mine.txt"), "not yours\n").unwrap();
    let mut asking = fixture.asking();
    asking.out = Some(taken.clone());

    let failure = package(&asking).unwrap_err();

    assert!(failure.to_string().contains("already exists"), "{failure}");
    assert!(taken.join("something-of-mine.txt").is_file());
}

/// A failure part way through has to leave the output path missing rather than
/// half-built, and it has to take the staging directory with it: a `.part` directory
/// left in a project is something its owner has to identify before they can delete it.
#[test]
fn a_failure_leaves_no_bundle_and_no_staging_directory() {
    let fixture = RunFixture::of(&one_survivor());
    let out = fixture.beside("into").join("bundle");
    let mut asking = fixture.asking();
    asking.out = Some(out.clone());
    // The snapshot is what a patch is checked against, and taking it away is a run
    // that cannot be packaged at all.
    fixture.remove("snapshot");

    let failure = package(&asking).unwrap_err();

    assert!(
        failure.to_string().contains("kept no snapshot"),
        "{failure}"
    );
    assert!(!out.exists(), "the output path was left behind");
    assert!(
        !fixture.beside("into").join("bundle.part").exists(),
        "the staging directory was left behind"
    );
}

/// Documents of two runs packaged as one would be read as a single measurement that
/// never happened.
#[test]
fn documents_belonging_to_another_run_are_refused() {
    for (document, key) in [("results.json", "run_id"), ("baseline.json", "run_id")] {
        let fixture = RunFixture::of(&one_survivor());
        let mut edited = fixture.document(document);
        edited[key] = json!("20260101T000000Z-abcdef");
        fixture.rewrite(document, &edited);

        let failure = package(&fixture.asking()).unwrap_err();

        let said = failure.to_string();
        assert!(
            said.contains("20260101T000000Z-abcdef"),
            "{document}: {said}"
        );
        assert!(
            said.contains("must not travel in one bundle"),
            "{document}: {said}"
        );
    }
}

/// A report stamped with another run is the same problem one document earlier, and it
/// is worth catching there: nothing else in the directory would be checked against
/// the right run afterwards.
#[test]
fn a_report_about_another_run_is_refused_before_anything_else_is_read() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("report.json");
    edited["run"]["run_id"] = json!("20260101T000000Z-abcdef");
    fixture.rewrite("report.json", &edited);

    let failure = package(&fixture.asking()).unwrap_err();

    assert!(failure.to_string().contains("is about run"), "{failure}");
    assert!(!fixture.default_out().exists());
}

/// One run's documents are one contract's. Packaging two versions together publishes
/// a bundle no single reader can be held to.
#[test]
fn documents_from_two_contract_versions_are_refused() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("results.json");
    edited["schema_version"] = json!("9.9");
    fixture.rewrite("results.json", &edited);

    let failure = package(&fixture.asking()).unwrap_err();

    let said = failure.to_string();
    assert!(said.contains("9.9"), "{said}");
    assert!(said.contains("0.1"), "{said}");
}

/// A triage is a claim about one report's verdicts. Beside another report it reads as
/// a judgement of verdicts it never saw.
#[test]
fn a_triage_about_another_report_is_refused() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.rewrite(
        "triage.json",
        &triage(
            &fixture.run_id,
            "report-from-somewhere-else.json",
            &fixture.ids[1],
        ),
    );

    let failure = package(&fixture.asking()).unwrap_err();

    assert!(
        failure
            .to_string()
            .contains("report-from-somewhere-else.json"),
        "{failure}"
    );
}

/// A triage of this run's own report travels, which is the case the refusal above has
/// to leave working.
#[test]
fn a_triage_of_this_report_is_packaged_beside_it() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.rewrite(
        "triage.json",
        &triage(&fixture.run_id, "report.json", &fixture.ids[1]),
    );

    let code = exit_status(&fixture.asking());

    assert_eq!(code, 0);
    let out = fixture.default_out();
    assert!(out.join("triage.json").is_file());
    assert!(fixture.index(&out).documents.triage.is_some());
}

/// A verdict about a mutant the manifest does not hold leaves nothing in the bundle
/// able to say what that verdict was about.
#[test]
fn a_report_and_a_manifest_covering_different_mutants_are_refused() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("manifest.json");
    edited["mutants"] = json!([edited["mutants"][0]]);
    fixture.rewrite("manifest.json", &edited);

    let failure = package(&fixture.asking()).unwrap_err();

    let said = failure.to_string();
    assert!(said.contains(&fixture.ids[1]), "{said}");
    assert!(said.contains("no entry in the manifest"), "{said}");
}

/// The other direction: a mutant nothing gave a verdict on would leave a reader
/// unable to account for it.
#[test]
fn a_manifest_holding_a_mutant_the_report_ignores_is_refused() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("report.json");
    edited["verdicts"] = json!([edited["verdicts"][0]]);
    fixture.rewrite("report.json", &edited);

    let failure = package(&fixture.asking()).unwrap_err();

    assert!(
        failure.to_string().contains("no verdict in its report"),
        "{failure}"
    );
}

/// A mutant identifier becomes a file path, so an identifier that is not one is a way
/// out of the bundle. The manifest holds it as a plain string, which is all a tampered
/// or hand-edited run needs.
#[test]
fn an_identifier_that_could_escape_the_bundle_is_refused() {
    for spelling in [
        "../../../../etc/passwd",
        "0e2f8c1d5a4b6e7f/../../8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e",
        "0E2F8C1D5A4B6E7F8091A2B3C4D5E6F708192A3B4C5D6E7F8091A2B3C4D5E6F7",
        "0e2f8c1d",
    ] {
        let fixture = RunFixture::of(&one_survivor());
        let mut manifest = fixture.document("manifest.json");
        manifest["mutants"][1]["id"] = json!(spelling);
        fixture.rewrite("manifest.json", &manifest);
        let mut report = fixture.document("report.json");
        report["verdicts"][1]["mutant_id"] = json!(spelling);
        fixture.rewrite("report.json", &report);

        let failure = package(&fixture.asking()).unwrap_err();

        assert!(
            failure
                .to_string()
                .contains("64 lowercase hexadecimal digits"),
            "{spelling}: {failure}"
        );
        assert!(!fixture.default_out().exists(), "{spelling}");
    }
}

/// A run of a manifest with nothing in it is a finished, successful run that happens
/// to have no evidence in it. Reporting it as a broken run would fail a run that did
/// exactly what it was asked to.
#[test]
fn a_run_that_tested_nothing_says_so_and_writes_no_bundle() {
    let fixture = RunFixture::nothing_to_test();

    let code = exit_status(&fixture.asking());

    assert_eq!(code, 0);
    assert!(
        !fixture.default_out().exists(),
        "a run with no evidence still produced a bundle"
    );
}

/// A run publishes `latest` before it writes its report, so a bundle asked for while a run
/// is going on finds a directory with no report in it. That is not a broken run and reading
/// it as an unreadable document says nothing a person can act on.
#[test]
fn a_run_with_no_report_yet_is_reported_as_one_that_has_not_finished() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.remove("report.json");

    let failure = package(&fixture.asking()).unwrap_err();

    let said = failure.to_string();
    assert!(said.contains("may still be in progress"), "{said}");
    assert!(said.contains("--run"), "{said}");
}

/// Every document agreeing with the report is not the same as the report being one this
/// tremula can read. A run whose documents all declare a version nothing here knows would
/// otherwise be packaged, and the bundle stamped with the version that packaged it.
#[test]
fn a_run_written_against_another_contract_is_refused_though_it_agrees_with_itself() {
    let fixture = RunFixture::of(&one_survivor());
    for document in [
        "report.json",
        "manifest.json",
        "results.json",
        "baseline.json",
    ] {
        let mut edited = fixture.document(document);
        edited["schema_version"] = json!("9.9");
        fixture.rewrite(document, &edited);
    }

    let failure = package(&fixture.asking()).unwrap_err();

    let said = failure.to_string();
    assert!(said.contains("9.9"), "{said}");
    assert!(said.contains("0.1"), "{said}");
    assert!(!fixture.default_out().exists());
}

/// A run's name carries the first six hex digits of the manifest it ran, which is the only
/// link between the two. A manifest swapped for another run's can hold the same mutants and
/// name no run of its own, so nothing else here would notice — and the spans and
/// replacements a reader would then read are not the ones the verdicts are about.
#[test]
fn a_manifest_that_is_not_the_one_the_run_ran_is_refused() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("manifest.json");
    // Every mutant left where it was, so this is the substitution the other checks pass.
    edited["base"] = json!({"revision": "9f1c0a3d5e7b2f4a6c8d0e2f4a6b8c0d2e4f6a80"});
    fixture.rewrite("manifest.json", &edited);

    let failure = package(&fixture.asking()).unwrap_err();

    let said = failure.to_string();
    assert!(said.contains("another run's manifest"), "{said}");
    assert!(said.contains(&fixture.run_id), "{said}");
    assert!(!fixture.default_out().exists());
}

/// There is no such run at all.
#[test]
fn a_run_directory_that_is_not_there_is_reported_as_missing() {
    let fixture = RunFixture::of(&one_survivor());
    let mut asking = fixture.asking();
    asking.run = Some(fixture.beside("no-such-run"));

    let failure = package(&asking).unwrap_err();

    assert!(
        failure.to_string().contains("no run directory"),
        "{failure}"
    );
}

/// A survivor is the one mutant a reader of the bundle has work to do about, so a
/// survivor whose patch git will not accept is the bundle failing at what it is for.
/// The bundle is still written — the rest of the evidence is real — and the exit code
/// is what says it is not the bundle that was asked for.
#[test]
fn a_survivor_whose_patch_does_not_apply_is_packaged_and_reported_as_a_failure() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("results.json");
    // A diff against a file the snapshot does not have those bytes for. This is the
    // shape of a run whose diff and whose snapshot came apart.
    edited["entries"][1]["diff"] = json!(format!(
        "--- a/{FILE}\n+++ b/{FILE}\n@@ -1,2 +1,2 @@\n-a line the file never had\n+another one\n"
    ));
    fixture.rewrite("results.json", &edited);

    let code = exit_status(&fixture.asking());

    assert_eq!(code, 2);
    let out = fixture.default_out();
    assert!(out.join("bundle.json").is_file(), "the bundle was withheld");
    let index = fixture.index(&out);
    let survivor = &index.attachments[1];
    assert!(survivor.patch.is_none());
    assert!(survivor.patch_error.is_some(), "{survivor:?}");
    assert!(
        !out.join("patches")
            .join(format!("{}.patch", survivor.id))
            .exists(),
        "a patch git refused was published anyway"
    );
}

/// A mutant the suite caught is not what a bundle is read for, so a patch of one that
/// does not apply is worth recording and not worth failing over.
#[test]
fn a_killed_mutant_whose_patch_does_not_apply_does_not_fail_the_bundle() {
    let fixture = RunFixture::of(&one_survivor());
    let mut edited = fixture.document("results.json");
    edited["entries"][0]["diff"] = json!(format!(
        "--- a/{FILE}\n+++ b/{FILE}\n@@ -1,2 +1,2 @@\n-a line the file never had\n+another one\n"
    ));
    fixture.rewrite("results.json", &edited);

    let code = exit_status(&fixture.asking());

    assert_eq!(code, 0);
    let index = fixture.index(&fixture.default_out());
    assert!(index.attachments[0].patch.is_none());
    assert!(index.attachments[0].patch_error.is_some());
}

/// A run whose pack recorded no diff at all: every patch is absent, and the reason is
/// in the document rather than only on the console that built it.
#[test]
fn a_run_with_no_diffs_says_why_each_patch_is_missing() {
    let fixture = RunFixture::without_diffs(&one_survivor());

    let code = exit_status(&fixture.asking());

    assert_eq!(code, 2, "a survivor with no patch is a failed bundle");
    let index = fixture.index(&fixture.default_out());
    for attachment in &index.attachments {
        assert!(attachment.patch.is_none());
        assert_eq!(
            attachment.patch_error.as_deref(),
            Some("the run recorded no diff for this mutant")
        );
    }
    assert!(!index.exposure.patch_context, "no patch, no source context");
}

/// The project's own name, and not the path the bundle was built at. A run records the
/// root as it was spelled on the command line, which is routinely `.` and names
/// nothing at all — which is why the command takes `--project`.
#[test]
fn the_bundle_names_the_project_and_not_the_path_it_was_built_at() {
    let fixture = RunFixture::of(&one_survivor());
    assert_eq!(
        fixture.document("report.json")["run"]["project"],
        json!("."),
        "the fixture has to be the case that makes this worth checking"
    );

    assert_eq!(exit_status(&fixture.asking()), 0);

    let index = fixture.index(&fixture.default_out());
    assert_eq!(index.project, PROJECT);
    let document = fs::read_to_string(fixture.default_out().join("bundle.json")).unwrap();
    assert!(
        !document.contains(&fixture.project().display().to_string()),
        "the bundle's index carries the path it was built at: {document}"
    );
}

/// A run with no verdicts and a manifest beside it is not the empty-manifest case: it
/// is a directory whose documents disagree, and it must not be read as a success.
#[test]
fn a_run_with_a_manifest_and_no_verdicts_is_not_read_as_nothing_to_package() {
    let fixture = RunFixture::of(&one_survivor());
    fixture.rewrite("report.json", &report(&fixture.run_id, &[]));

    let failure = package(&fixture.asking()).unwrap_err();

    assert!(
        failure.to_string().contains("no verdict in its report"),
        "{failure}"
    );
}
