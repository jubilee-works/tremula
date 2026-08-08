"""One neutral reading for every signal the backend can hand back.

The translation is a table, and so is this: each row is a state a job can be found
in, and the status it has to become. Two rows deserve their reputation. A run the
runner stopped is a timeout *before* anything is asked about hashes or markers,
because the runner records the hashes before the suite starts and a killed run
therefore still carries them. And an attempt whose marker never arrived is a
backend error with no runner block at all, rather than one full of zeros.

The states are reached by overwriting one job's result in a finished run's session,
which is the only way to observe a segfaulting worker or a Cosmic Ray timeout
without arranging for one.
"""

import json
from collections.abc import Callable
from pathlib import Path
from shutil import copytree
from typing import Any

import pytest
from cosmic_ray.work_item import (  # pyright: ignore[reportMissingTypeStubs]
    TestOutcome as Outcome,  # aliased: pytest would try to collect a `Test`-prefixed name
)
from cosmic_ray.work_item import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    WorkerOutcome,
    WorkResult,
)

from tremula_python import collect, engine
from tremula_python.__main__ import main
from tremula_python.contracts import Manifest, ResultEntry
from tremula_python.cr_operator import FULL_OPERATOR_NAME
from tremula_python.engine import MUTANT_ID_ARGUMENT
from tremula_python.pytest_runner import MARKER_PREFIX

FIELDS: dict[str, Any] = {
    "passed": 3,
    "failed": 0,
    "errors": 0,
    "skipped": 0,
    "collected": 3,
    "collected_ids_hash": "b5c7d9",
    "collect_error": False,
    "timed_out": False,
    "pytest_exit": 0,
    "duration_ms": 1240,
    "target_hashes": {},
}


def _marker(**overrides: Any) -> str:
    return MARKER_PREFIX + json.dumps({**FIELDS, **overrides})


def _marker_without_hashes() -> str:
    """A marker that never mentions the file hashes."""
    return MARKER_PREFIX + json.dumps(
        {key: value for key, value in FIELDS.items() if key != "target_hashes"}
    )


@pytest.fixture(scope="module")
def finished_run(
    tmp_path_factory: pytest.TempPathFactory,
    copy_pack_project: Callable[[Path], Path],
    pack_project_manifest: dict[str, Any],
) -> Path:
    """One real run of the fixture project, to take results apart afterwards."""
    workspace = tmp_path_factory.mktemp("translation")
    project = copy_pack_project(workspace / "project")
    manifest = workspace / "manifest.json"
    manifest.write_text(json.dumps(pack_project_manifest), encoding="utf-8")
    run_dir = workspace / "20260808T160000Z-5e7b2f"

    assert (
        main(
            [
                "run",
                "--manifest",
                str(manifest),
                "--project",
                str(project),
                "--out",
                str(run_dir),
            ]
        )
        == 0
    )
    return run_dir


@pytest.fixture
def run_copy(finished_run: Path, tmp_path: Path) -> Path:
    destination = tmp_path / finished_run.name
    copytree(finished_run, destination)
    return destination


def _first_entry_after(run_dir: Path, result: WorkResult) -> ResultEntry:
    """Replace the first mutant's job result with `result`, then collect."""
    manifest = Manifest.model_validate_json((run_dir / "manifest.json").read_bytes())
    target = manifest.mutants[0]
    with engine.open_session(run_dir / "session.sqlite") as session:
        for item in session.work_items:
            mutation = item.mutations[0]
            if (
                mutation.operator_name == FULL_OPERATOR_NAME
                and mutation.operator_args.get(MUTANT_ID_ARGUMENT) == target.id
                and mutation.module_path.as_posix() == target.file
            ):
                session.set_result(item.job_id, result)
                break
        else:
            raise AssertionError("the finished run has no job for its first mutant")
    return collect.collect(run_dir).entries[0]


@pytest.mark.parametrize(
    ("description", "result", "status", "has_runner"),
    [
        (
            "a job the filter set aside",
            WorkResult(worker_outcome=WorkerOutcome.SKIPPED),
            "skipped",
            False,
        ),
        (
            "the backend found no mutation to make",
            WorkResult(worker_outcome=WorkerOutcome.NO_TEST),
            "not_applied",
            False,
        ),
        (
            "the worker raised",
            WorkResult(
                worker_outcome=WorkerOutcome.EXCEPTION,
                output="Traceback (most recent call last): ...",
                test_outcome=Outcome.INCOMPETENT,
            ),
            "backend_error",
            False,
        ),
        (
            "the worker died without raising",
            WorkResult(worker_outcome=WorkerOutcome.ABNORMAL, output="killed by signal 11"),
            "backend_error",
            False,
        ),
        (
            "the backend's own time limit fired, leaving no marker",
            WorkResult(
                worker_outcome=WorkerOutcome.NORMAL,
                output="timeout",
                test_outcome=Outcome.KILLED,
            ),
            "timeout",
            False,
        ),
        (
            "the runner stopped the suite itself and said so",
            WorkResult(
                worker_outcome=WorkerOutcome.NORMAL,
                output=_marker(timed_out=True, pytest_exit=-9),
                test_outcome=Outcome.KILLED,
            ),
            "timeout",
            True,
        ),
        (
            "the suite ran but the runner printed no marker",
            WorkResult(
                worker_outcome=WorkerOutcome.NORMAL,
                output="collected 3 items\n3 passed in 0.10s\n",
                test_outcome=Outcome.SURVIVED,
            ),
            "backend_error",
            False,
        ),
        (
            "the marker arrived with the file hashes empty",
            WorkResult(
                worker_outcome=WorkerOutcome.NORMAL,
                output=_marker(),
                test_outcome=Outcome.SURVIVED,
            ),
            "backend_error",
            True,
        ),
        (
            # Not empty — absent. An older runner, or one that could not read the
            # target list. The suite still ran and still reported, so the entry
            # keeps its runner block and fails on the evidence, not on the marker.
            "the marker arrived with no file hashes field at all",
            WorkResult(
                worker_outcome=WorkerOutcome.NORMAL,
                output=_marker_without_hashes(),
                test_outcome=Outcome.SURVIVED,
            ),
            "backend_error",
            True,
        ),
    ],
)
def test_each_backend_signal_has_one_neutral_reading(
    run_copy: Path, description: str, result: WorkResult, status: str, has_runner: bool
) -> None:
    entry = _first_entry_after(run_copy, result)

    assert entry.execution_status.value == status, description
    assert (entry.runner is not None) == has_runner, description


def test_a_stopped_run_is_a_timeout_even_though_its_hashes_are_missing(
    run_copy: Path,
) -> None:
    # The precedence that matters: the runner writes the hashes before the suite
    # starts, so a killed run can carry a marker with nothing in it. Reading that
    # as a hash problem would report a timeout as a backend defect.
    entry = _first_entry_after(
        run_copy,
        WorkResult(
            worker_outcome=WorkerOutcome.NORMAL,
            output=_marker(timed_out=True, pytest_exit=124, target_hashes={}),
            test_outcome=Outcome.KILLED,
        ),
    )

    assert entry.execution_status.value == "timeout"
    assert entry.runner is not None
    # The runner block is kept for the diagnosis it carries, and a stopped run is
    # interrupted whatever number the process exited with.
    assert (entry.runner.timed_out, entry.runner.exit_class.value) == (True, "interrupted")


def test_the_backends_vocabulary_survives_translation(run_copy: Path) -> None:
    entry = _first_entry_after(
        run_copy,
        WorkResult(
            worker_outcome=WorkerOutcome.EXCEPTION,
            output="Traceback: operator blew up",
            test_outcome=Outcome.INCOMPETENT,
            diff="--- amutated\n+++ bmutated\n",
        ),
    )

    assert entry.backend_raw == {
        "worker_outcome": "exception",
        "test_outcome": "incompetent",
        "output": "Traceback: operator blew up",
        "diff": "--- amutated\n+++ bmutated\n",
        "pytest_exit": None,
    }
