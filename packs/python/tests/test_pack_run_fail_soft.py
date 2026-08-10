"""One impossible mutant, and a batch that survives it.

After a manifest has been accepted there are three points at which a single
mutant can still turn out to be unusable: the language checks, the session file
the backend reads its parameters out of, and the backend's own count of the jobs
it made from them. Each of those used to end the run, and a run that ends there
writes no results document at all — so a measured sweep lost the verdicts of 33
mutants to one that could not be applied.

Now the refused mutant is left out and named. The rest run and their verdicts are
real; the refused one appears in the results document as a mutation that was never
applied, carrying the reason, which is what turns a run like this into exit 2 in
the core rather than a quiet success. Preserving the batch is the point and
concealing the refusal is not: every refusal is in the document, in the manifest's
own order, and in a log of its own.

The three shapes are the ones a measured sweep actually produced — a replacement
that only compiles on its own, a docstring whose quotes the session file's encoder
eats, and an annotated parameter the backend matches twice.
"""

import json
import subprocess
import sys
from collections.abc import Callable
from hashlib import sha256
from pathlib import Path
from typing import Any

import pytest

from tremula_python.contracts import Results

RAN = ("caught-by-the-suite", "missed-by-the-suite")
"""The fixture project's golden pair: one mutant the suite catches, one it misses."""

REFUSED_AT_VALIDATE = "will-not-compile"
REFUSED_AT_THE_SESSION_FILE = "cannot-travel-as-toml"
REFUSED_BY_THE_JOB_COUNT = "matched-twice"

NEEDS_BREAK_DOCSTRING = (
    '"""Whether a meeting that long should have a break scheduled after it."""'
)


def _mutant(source: str, name: str, original: str, replacement: str) -> dict[str, Any]:
    """One mutant of `schedule.py`, named so a failure can be read by its name.

    The identifiers are not the canonical derivation, which the pack never checks
    and the core owns: what matters here is which mutant a message is about.
    """
    start = source.index(original)
    return {
        "id": name,
        "file": "schedule.py",
        "base_file_sha256": sha256(source.encode("utf-8")).hexdigest(),
        "span": {"start_byte": start, "end_byte": start + len(original.encode("utf-8"))},
        "original": original,
        "replacement": replacement,
    }


def _document(*mutants: dict[str, Any]) -> str:
    return json.dumps(
        {
            "schema_version": "0.1",
            "language": "python",
            "base": {"revision": None},
            "mutants": list(mutants),
        }
    )


def _pack_run(project: Path, manifest: Path, run_dir: Path) -> subprocess.CompletedProcess[str]:
    """Run the pack the way the core does: as a module, in this interpreter."""
    return subprocess.run(
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


@pytest.fixture(scope="module")
def mixed_run(
    tmp_path_factory: pytest.TempPathFactory,
    copy_pack_project: Callable[[Path], Path],
) -> tuple[subprocess.CompletedProcess[str], Path]:
    """The fixture project, run once with a manifest three of whose five mutants fail."""
    workspace = tmp_path_factory.mktemp("fail-soft")
    project: Path = copy_pack_project(workspace / "project")
    source = (project / "schedule.py").read_text(encoding="utf-8")
    manifest = workspace / "manifest.json"
    manifest.write_text(
        _document(
            _mutant(source, RAN[0], "other_start < end", "other_start <= end"),
            _mutant(source, REFUSED_AT_VALIDATE, "start < other_end", "start <= "),
            _mutant(source, RAN[1], "minutes >= 60", "minutes > 60"),
            _mutant(
                source,
                REFUSED_AT_THE_SESSION_FILE,
                NEEDS_BREAK_DOCSTRING,
                '"""Something else entirely."""',
            ),
            _mutant(source, REFUSED_BY_THE_JOB_COUNT, "minutes: int", "minutes: float"),
        ),
        encoding="utf-8",
    )
    run_dir = workspace / "20260810T091500Z-3b1f8c"

    return _pack_run(project, manifest, run_dir), run_dir


def _entries(run_dir: Path) -> dict[str, Any]:
    results = Results.model_validate_json((run_dir / "results.json").read_bytes())
    return {entry.mutant_id: entry for entry in results.entries}


def test_the_run_finishes_and_reports_every_mutant_it_was_given(
    mixed_run: tuple[subprocess.CompletedProcess[str], Path],
) -> None:
    completed, run_dir = mixed_run

    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert (completed.stdout, completed.stderr) == ("", "")
    assert list(_entries(run_dir)) == [
        RAN[0],
        REFUSED_AT_VALIDATE,
        RAN[1],
        REFUSED_AT_THE_SESSION_FILE,
        REFUSED_BY_THE_JOB_COUNT,
    ]


def test_the_mutants_that_could_be_applied_have_real_verdicts(
    mixed_run: tuple[subprocess.CompletedProcess[str], Path],
) -> None:
    # This is what fail-soft is for. Both of these used to be lost — no results
    # document written at all — because of mutants neither of them touches.
    _, run_dir = mixed_run
    entries = _entries(run_dir)

    caught = entries[RAN[0]]
    assert caught.execution_status.value == "completed"
    assert caught.runner is not None
    assert caught.runner.failed == 1

    missed = entries[RAN[1]]
    assert missed.execution_status.value == "completed"
    assert missed.runner is not None
    assert (missed.runner.failed, missed.runner.errors) == (0, 0)
    assert missed.runner.exit_class.value == "ok"


@pytest.mark.parametrize(
    ("mutant_id", "code"),
    [
        (REFUSED_AT_VALIDATE, "replacement_does_not_compile"),
        (REFUSED_AT_THE_SESSION_FILE, "unserializable_mutant"),
        (REFUSED_BY_THE_JOB_COUNT, "mutant_job_mismatch"),
    ],
)
def test_a_refused_mutant_is_never_applied_and_says_why(
    mixed_run: tuple[subprocess.CompletedProcess[str], Path], mutant_id: str, code: str
) -> None:
    _, run_dir = mixed_run

    entry = _entries(run_dir)[mutant_id]

    assert entry.execution_status.value == "not_applied"
    assert entry.runner is None
    refusal: dict[str, Any] = entry.backend_raw["refusal"]
    assert refusal["code"] == code
    assert mutant_id in refusal["message"]
    # And in a log of its own, which is where a reader looks first.
    kept = (run_dir / "logs" / f"{mutant_id}.txt").read_text(encoding="utf-8")
    assert refusal["message"] in kept


def test_a_manifest_the_backend_counts_wrongly_from_end_to_end_stops_the_run(
    tmp_path: Path, copy_pack_project: Callable[[Path], Path]
) -> None:
    # The three refusal points are three chances to empty a batch, and the last one
    # empties it as thoroughly as the first: every job the session holds has been
    # marked skipped, so there is nothing left to execute and no verdict to collect.
    # A run that reported itself finished here would be reporting a session it had
    # just emptied.
    project = copy_pack_project(tmp_path / "project")
    source = (project / "schedule.py").read_text(encoding="utf-8")
    manifest = tmp_path / "manifest.json"
    manifest.write_text(
        _document(_mutant(source, REFUSED_BY_THE_JOB_COUNT, "minutes: int", "minutes: float")),
        encoding="utf-8",
    )
    run_dir = tmp_path / "20260810T093000Z-3b1f8c"

    completed = _pack_run(project, manifest, run_dir)

    assert (completed.returncode, completed.stderr) == (2, "")
    error: dict[str, Any] = json.loads(completed.stdout.splitlines()[-1])["error"]
    assert (error["stage"], error["code"]) == ("plan", "every_mutant_refused")
    assert REFUSED_BY_THE_JOB_COUNT in error["message"]
    # And the record of what was refused is on disk before the run gives up, so the
    # run directory says why without the error document in front of the reader.
    refused: dict[str, Any] = json.loads((run_dir / "refusals.json").read_text(encoding="utf-8"))
    assert refused[REFUSED_BY_THE_JOB_COUNT]["code"] == "mutant_job_mismatch"


def test_a_manifest_none_of_whose_mutants_can_be_applied_stops_the_run(
    tmp_path: Path, copy_pack_project: Callable[[Path], Path]
) -> None:
    # Fail-soft saves the rest of a batch, and here there is no rest: no session
    # would be built and no results document could be written, so a run that
    # reported itself finished would be concealing that nothing had happened.
    project = copy_pack_project(tmp_path / "project")
    source = (project / "schedule.py").read_text(encoding="utf-8")
    manifest = tmp_path / "manifest.json"
    manifest.write_text(
        _document(
            _mutant(source, "will-not-compile", "start < other_end", "start <= "),
            _mutant(source, "regroups-only", "minutes >= 60", "(minutes >= 60)"),
        ),
        encoding="utf-8",
    )

    completed = _pack_run(project, manifest, tmp_path / "20260810T092000Z-3b1f8c")

    assert (completed.returncode, completed.stderr) == (2, "")
    error: dict[str, Any] = json.loads(completed.stdout.splitlines()[-1])["error"]
    assert error["code"] == "every_mutant_refused"
    assert "will-not-compile" in error["message"]
    assert "regroups-only" in error["message"]
