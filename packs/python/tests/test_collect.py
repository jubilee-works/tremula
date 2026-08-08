"""Reading a run's results back out of its directory.

Collecting is the recovery path as much as the reporting one: it takes nothing but
a directory, it has to work on a run that never finished, and running it twice has
to produce the same document. The cases here cover each of those, plus the two
ways the reading can go wrong — a directory that is not a run, and files that were
not what the mutation should have made of them.
"""

import json
from collections.abc import Callable
from hashlib import sha256
from pathlib import Path
from shutil import copytree
from typing import Any

import pytest
from conftest import MutantIdentifier

from tremula_python import collect, engine, plan_session
from tremula_python.__main__ import main
from tremula_python.contracts import Manifest
from tremula_python.errors import PackFailure
from tremula_python.run_layout import RunLayout


@pytest.fixture(scope="module")
def finished_run(
    tmp_path_factory: pytest.TempPathFactory,
    copy_pack_project: Callable[[Path], Path],
    pack_project_manifest: dict[str, Any],
) -> Path:
    """One real run of the fixture project, shared by every case that needs one."""
    workspace = tmp_path_factory.mktemp("collect")
    project = copy_pack_project(workspace / "project")
    manifest = workspace / "manifest.json"
    manifest.write_text(json.dumps(pack_project_manifest), encoding="utf-8")
    run_dir = workspace / "20260808T130000Z-9f1c0a"

    assert main(
        [
            "run",
            "--manifest",
            str(manifest),
            "--project",
            str(project),
            "--out",
            str(run_dir),
        ]
    ) == 0
    return run_dir


@pytest.fixture
def run_copy(finished_run: Path, tmp_path: Path) -> Path:
    """A private copy of the finished run, safe to take apart."""
    destination = tmp_path / finished_run.name
    copytree(finished_run, destination)
    return destination


def _results(run_dir: Path) -> dict[str, Any]:
    document: dict[str, Any] = json.loads((run_dir / "results.json").read_text(encoding="utf-8"))
    return document


def _statuses(run_dir: Path) -> list[str]:
    return [entry["execution_status"] for entry in _results(run_dir)["entries"]]


def test_collecting_twice_produces_the_same_document(run_copy: Path) -> None:
    # Byte for byte, not merely equivalent: an interrupted run is collected again,
    # and a document that changed on its own would make the two indistinguishable
    # from a run whose results changed.
    first = (run_copy / "results.json").read_bytes()

    collect.collect(run_copy)

    assert (run_copy / "results.json").read_bytes() == first


def test_the_collect_subcommand_rebuilds_the_document_in_place(
    run_copy: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    first = (run_copy / "results.json").read_bytes()
    (run_copy / "results.json").unlink()

    assert main(["collect", "--out", str(run_copy)]) == 0

    assert (run_copy / "results.json").read_bytes() == first
    assert capsys.readouterr().out == ""


def test_every_mutant_appears_once_in_manifest_order(run_copy: Path) -> None:
    manifest = Manifest.model_validate_json((run_copy / "manifest.json").read_bytes())

    entries = _results(run_copy)["entries"]

    assert [entry["mutant_id"] for entry in entries] == [m.id for m in manifest.mutants]


@pytest.mark.parametrize(
    "removed", ["manifest.json", "expected-hashes.json", "diffs.json", "session.sqlite"]
)
def test_a_directory_that_is_not_a_run_is_refused(run_copy: Path, removed: str) -> None:
    (run_copy / removed).unlink()

    with pytest.raises(PackFailure) as raised:
        collect.collect(run_copy)

    assert (raised.value.stage.value, raised.value.code) == ("collect", "incomplete_run_dir")
    assert removed in raised.value.message


def test_a_directory_that_does_not_exist_is_refused(tmp_path: Path) -> None:
    with pytest.raises(PackFailure) as raised:
        collect.collect(tmp_path / "never-ran")

    assert raised.value.code == "incomplete_run_dir"


def test_a_file_that_is_not_what_the_mutation_makes_of_it_is_a_backend_error(
    run_copy: Path,
) -> None:
    # The comparison is the pack's only evidence that the mutation it asked for is
    # the mutation that ran, so a table it disagrees with has to be fatal to the
    # entry rather than passed over.
    hashes = plan_session.read_expected_hashes(run_copy / "expected-hashes.json")
    tampered = {"base": hashes.base, "mutated": dict.fromkeys(hashes.mutated, "0" * 64)}
    (run_copy / "expected-hashes.json").write_text(json.dumps(tampered), encoding="utf-8")

    collect.collect(run_copy)

    assert _statuses(run_copy) == ["backend_error", "backend_error"]
    entries = _results(run_copy)["entries"]
    # The runner still reported; what failed is the pack's check on it.
    assert entries[0]["runner"] is not None
    assert "hashed" in (run_copy / "logs" / f"{entries[0]['mutant_id']}.txt").read_text(
        encoding="utf-8"
    )


def test_a_target_file_that_changed_underneath_the_run_is_a_backend_error(
    run_copy: Path,
) -> None:
    hashes = plan_session.read_expected_hashes(run_copy / "expected-hashes.json")
    tampered = {
        "base": {"another.py": "0" * 64, **hashes.base},
        "mutated": hashes.mutated,
    }
    (run_copy / "expected-hashes.json").write_text(json.dumps(tampered), encoding="utf-8")

    collect.collect(run_copy)

    assert _statuses(run_copy) == ["backend_error", "backend_error"]


def test_an_attempt_the_backend_never_ran_is_reported_as_not_run(
    tmp_path: Path,
    copy_pack_project: Callable[[Path], Path],
    pack_project_manifest: dict[str, Any],
) -> None:
    # The state an interrupted run leaves behind: a planned, filtered session with
    # nothing executed. Every mutant still needs an entry, because a report that
    # simply omitted them would look complete.
    project = copy_pack_project(tmp_path / "project")
    manifest_text = json.dumps(pack_project_manifest)
    manifest = Manifest.model_validate_json(manifest_text)
    layout = RunLayout.at(tmp_path / "20260808T140000Z-000000")
    layout.prepare()
    plan_session.build_plan(manifest, project, layout, [], 40.0).write(layout, manifest_text)
    engine.init_session(layout, project)
    engine.filter_jobs(layout.session, manifest)

    results = collect.collect(layout.directory)

    assert [entry.execution_status.value for entry in results.entries] == ["not_run", "not_run"]
    for entry in results.entries:
        assert entry.runner is None
        assert entry.backend_raw == {}
        # Where the mutant would have landed is known from the job, even though
        # nothing ran; what happened to it is not.
        assert entry.location is not None
        assert entry.finished_at is None


def test_a_mutant_with_no_job_at_all_is_reported_as_not_applied(
    run_copy: Path, canonical_mutant_id: MutantIdentifier
) -> None:
    # A mutant the backend never made a job for. The filter refuses to start a run
    # in that state, so reaching collect means an interrupted run whose session
    # predates the manifest — and an entry is still owed for it.
    document = json.loads((run_copy / "manifest.json").read_text(encoding="utf-8"))
    stranger = dict(document["mutants"][1])
    stranger["replacement"] = "minutes > 90"
    stranger["id"] = canonical_mutant_id(
        stranger["file"],
        stranger["span"]["start_byte"],
        stranger["span"]["end_byte"],
        stranger["base_file_sha256"],
        stranger["replacement"],
    )
    document["mutants"].append(stranger)
    (run_copy / "manifest.json").write_text(json.dumps(document), encoding="utf-8")

    results = collect.collect(run_copy)

    assert [entry.execution_status.value for entry in results.entries] == [
        "completed",
        "completed",
        "not_applied",
    ]
    assert results.entries[2].location is None
    assert results.entries[2].diff is None


def test_a_filtered_duplicate_never_displaces_the_attempt_that_ran(
    tmp_path: Path, canonical_mutant_id: MutantIdentifier
) -> None:
    # Two files holding the same code at the same position: each mutant matches in
    # both, so each has a skipped twin in the session. The entry has to come from
    # the job for the mutant's own file.
    project = tmp_path / "project"
    project.mkdir()
    source = "def small(value: int) -> bool:\n    return value < 2\n"
    for name in ("first.py", "second.py"):
        (project / name).write_text(source, encoding="utf-8")
    (project / "test_both.py").write_text(
        "from first import small as one\n"
        "from second import small as two\n"
        "\n"
        "def test_one() -> None:\n"
        "    assert one(1)\n"
        "\n"
        "def test_two() -> None:\n"
        "    assert two(1)\n",
        encoding="utf-8",
    )
    document = {
        "schema_version": "0.1",
        "language": "python",
        "base": {"revision": None},
        "mutants": [
            _twin_mutant(name, source, canonical_mutant_id)
            for name in ("first.py", "second.py")
        ],
    }
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(json.dumps(document), encoding="utf-8")
    run_dir = tmp_path / "20260808T150000Z-abcdef"

    assert main(
        [
            "run",
            "--manifest",
            str(manifest_path),
            "--project",
            str(project),
            "--out",
            str(run_dir),
        ]
    ) == 0

    assert _statuses(run_dir) == ["completed", "completed"]


def _twin_mutant(file: str, source: str, identify: MutantIdentifier) -> dict[str, Any]:
    """The same mutation in whichever of the two identical files is named."""
    original = "value < 2"
    start = source.index(original)
    end = start + len(original)
    digest = sha256(source.encode("utf-8")).hexdigest()
    replacement = "value <= 2"
    return {
        "id": identify(file, start, end, digest, replacement),
        "file": file,
        "base_file_sha256": digest,
        "span": {"start_byte": start, "end_byte": end},
        "original": original,
        "replacement": replacement,
    }
