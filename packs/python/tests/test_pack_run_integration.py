"""One manifest, all the way through a real Cosmic Ray run.

This is the case the whole pack exists to make work: a project with a suite that
is tight in one place and loose in another, two mutants aimed at each, and a
results document that says — in neutral terms and without judging anything — that
one mutation made the suite fail and the other did not.

It runs the pack the way the core does, as a subprocess of this interpreter, so
the exit code and the discipline about stdout are under test too.
"""

import json
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest
from conftest import MutantIdentifier

from tremula_python.contracts import Baseline, Results

CONTRACTS = Path(__file__).resolve().parents[3] / "contracts"


def _schema(name: str) -> dict[str, Any]:
    return json.loads((CONTRACTS / "schemas" / name).read_text(encoding="utf-8"))


def test_the_identifier_derivation_agrees_with_the_core(
    canonical_mutant_id: MutantIdentifier,
) -> None:
    # The same vector the Rust suite pins, computed here. The core rejects any
    # manifest whose identifiers are not this derivation, so the two
    # implementations agreeing is what makes a pack-built manifest usable at all.
    assert canonical_mutant_id(
        "src/scheduling/overlap.py",
        412,
        431,
        "7d2e4f6a8b0c2d4e6f8a0b2c4d6e8f0a2b4c6d8e0f2a4b6c8d0e2f4a6b8c0d2e",
        "start <= other.end",
    ) == "7c00b65a07e206cc7a472043b4a74359ca09e404db1dc431b06bda48e80ca829"


@pytest.fixture(scope="module")
def finished_run(
    tmp_path_factory: pytest.TempPathFactory,
    copy_pack_project: Callable[[Path], Path],
    pack_project_manifest: dict[str, Any],
) -> tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]]:
    """The fixture project, run through the pack once, as the core would run it."""
    workspace = tmp_path_factory.mktemp("integration")
    project: Path = copy_pack_project(workspace / "project")
    manifest = workspace / "manifest.json"
    manifest.write_text(json.dumps(pack_project_manifest), encoding="utf-8")
    run_dir = workspace / "20260808T121500Z-3b1f8c"

    completed = subprocess.run(
        [
            sys.executable,
            "-m",
            "tremula_python",
            "run",
            "--manifest",
            str(manifest),
            "--project",
            str(project),
            "--out",
            str(run_dir),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    return completed, run_dir, pack_project_manifest


def _results(run_dir: Path) -> Results:
    return Results.model_validate_json((run_dir / "results.json").read_bytes())


def test_the_run_succeeds_and_says_nothing_on_the_way(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    completed, _, _ = finished_run

    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert completed.stderr == ""
    assert completed.stdout == ""


def test_the_run_leaves_every_document_a_run_owes(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    _, run_dir, manifest = finished_run

    for name in ("manifest.json", "config.toml", "session.sqlite", "baseline.json"):
        assert (run_dir / name).is_file(), name
    assert json.loads((run_dir / "manifest.json").read_text(encoding="utf-8")) == manifest
    for name in ("init.txt", "exec.txt", "baseline.txt"):
        assert (run_dir / "logs" / name).is_file(), name


def test_the_results_document_satisfies_the_contract(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    _, run_dir, _ = finished_run

    document = json.loads((run_dir / "results.json").read_text(encoding="utf-8"))

    jsonschema.validate(instance=document, schema=_schema("results.schema.json"))
    assert document["run_id"] == run_dir.name
    assert len(document["entries"]) == 2


def test_the_mutant_the_suite_pins_makes_it_fail(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    _, run_dir, manifest = finished_run

    entry = _results(run_dir).entries[0]

    assert entry.mutant_id == manifest["mutants"][0]["id"]
    assert entry.execution_status.value == "completed"
    assert entry.runner is not None
    assert entry.runner.failed == 1
    assert entry.runner.exit_class.value == "test_failures"


def test_the_mutant_nothing_pins_survives_a_full_suite(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    _, run_dir, manifest = finished_run
    reference = Baseline.model_validate_json((run_dir / "baseline.json").read_bytes())

    entry = _results(run_dir).entries[1]

    assert entry.mutant_id == manifest["mutants"][1]["id"]
    assert entry.execution_status.value == "completed"
    assert entry.runner is not None
    assert (entry.runner.failed, entry.runner.errors) == (0, 0)
    assert entry.runner.passed == reference.runner.passed
    # Not just the same count: the same tests. A mutant that broke collection
    # could otherwise pass for a survivor.
    assert entry.runner.collected_ids_hash == reference.runner.collected_ids_hash
    assert entry.runner.exit_class.value == "ok"


def test_every_entry_keeps_the_backends_own_account(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    _, run_dir, _ = finished_run

    for entry in _results(run_dir).entries:
        assert entry.backend_raw["worker_outcome"] == "normal"
        assert entry.backend_raw["test_outcome"] in {"killed", "survived"}
        assert isinstance(entry.backend_raw["pytest_exit"], int)
        assert "TREMULA-RESULT" in str(entry.backend_raw["output"])


def test_every_entry_carries_a_standard_diff_and_a_location(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    _, run_dir, manifest = finished_run
    source = (run_dir.parent / "project" / "schedule.py").read_text(encoding="utf-8")

    for entry, mutant in zip(_results(run_dir).entries, manifest["mutants"], strict=True):
        assert entry.diff is not None
        lines = entry.diff.splitlines()
        assert lines[0] == "--- a/schedule.py"
        assert lines[1] == "+++ b/schedule.py"
        body = lines[3:]
        assert any(mutant["original"] in line for line in body if line.startswith("-"))
        assert any(mutant["replacement"] in line for line in body if line.startswith("+"))
        assert entry.location is not None
        # The location is the line the mutant really landed on, counted from one.
        assert source.splitlines()[entry.location.line - 1].endswith(mutant["original"])


def test_the_diff_of_every_entry_applies_to_the_project(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
    tmp_path: Path,
    copy_pack_project: Callable[[Path], Path],
) -> None:
    # The point of a diff in the report is that a reader can use it, and a hunk
    # over a span's raw bytes cannot be used: a manifest span routinely begins and
    # ends mid-line. `git apply --check` is the arbiter.
    _, run_dir, _ = finished_run
    project = copy_pack_project(tmp_path / "unmutated")

    for entry in _results(run_dir).entries:
        assert entry.diff is not None
        patch = tmp_path / f"{entry.mutant_id}.diff"
        patch.write_text(entry.diff, encoding="utf-8")

        checked = subprocess.run(
            ["git", "apply", "--check", str(patch)],
            cwd=project,
            capture_output=True,
            text=True,
            check=False,
        )

        assert checked.returncode == 0, f"{entry.mutant_id}: {checked.stderr}\n{entry.diff}"


def test_no_attempt_claims_a_time_it_does_not_know(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    # The session records no clock time, so `finished_at` stays null rather than
    # being filled in with the moment the results happened to be assembled.
    _, run_dir, _ = finished_run

    for entry in _results(run_dir).entries:
        assert entry.finished_at is None
        assert entry.truncated is False


def test_each_mutants_own_test_output_is_kept(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    _, run_dir, manifest = finished_run

    for mutant in manifest["mutants"]:
        log = (run_dir / "logs" / f"{mutant['id']}.txt").read_text(encoding="utf-8")
        assert "test_schedule.py" in log


def test_the_project_is_left_as_it_was_found(
    finished_run: tuple[subprocess.CompletedProcess[str], Path, dict[str, Any]],
) -> None:
    # Mutations are applied in place, so a run that finishes has to have put every
    # file back — and left no bytecode behind to shadow the sources next time.
    _, run_dir, manifest = finished_run
    project = run_dir.parent / "project"

    source = (project / "schedule.py").read_text(encoding="utf-8")

    assert manifest["mutants"][0]["original"] in source
    assert manifest["mutants"][1]["original"] in source
    assert list(project.rglob("__pycache__")) == []
