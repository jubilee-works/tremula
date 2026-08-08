"""Driving Cosmic Ray, and cutting a fresh session down to one job per mutant.

Every case here runs the real `cosmic-ray init`, because what is under test is
the shape of the session it produces: a built-in operator is instantiated whether
or not the configuration mentions it, and this pack's operator is a pattern
matcher that cannot tell one file from another. The filter is what turns that
into a one-to-one correspondence with the manifest — and therefore what makes a
missing job mean a real bug rather than an ordinary miss.
"""

import json
from hashlib import sha256
from pathlib import Path
from typing import Any

import pytest

from tremula_python import engine, plan_session
from tremula_python.contracts import Manifest
from tremula_python.cr_operator import FULL_OPERATOR_NAME
from tremula_python.errors import PackFailure
from tremula_python.run_layout import RunLayout

SOURCE = "def overlaps(start, end):\n    return start < end\n"
TARGET = "src/overlap.py"
TWIN = "src/twin.py"


def _mutant(
    identifier: str,
    *,
    file: str = TARGET,
    located: str = "start < end",
    original: str | None = None,
    replacement: str = "start <= end",
) -> dict[str, Any]:
    """A mutant over `SOURCE`.

    `original` defaults to the text the span really covers; giving it separately
    is how a mutant that cannot match anything is described.
    """
    start = SOURCE.index(located)
    return {
        "id": identifier,
        "file": file,
        "base_file_sha256": sha256(SOURCE.encode("utf-8")).hexdigest(),
        "span": {"start_byte": start, "end_byte": start + len(located.encode("utf-8"))},
        "original": located if original is None else original,
        "replacement": replacement,
    }


def _document(*mutants: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": "0.1",
        "language": "python",
        "base": {"revision": None},
        "mutants": list(mutants),
    }


def _planned(tmp_path: Path, *mutants: dict[str, Any]) -> tuple[RunLayout, Manifest, Path]:
    """A project holding every target file, and a plan written for it."""
    document = _document(*mutants)
    manifest = Manifest.model_validate(document)
    project = tmp_path / "project"
    for relative in plan_session.target_files(manifest):
        path = project / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(SOURCE, encoding="utf-8")
    layout = RunLayout.at(tmp_path / "20260808T120000Z-3b1f8c")
    layout.prepare()
    plan_session.build_plan(manifest, project, layout, [], 40.0).write(
        layout, json.dumps(document)
    )
    return layout, manifest, project


def _initialized(tmp_path: Path, *mutants: dict[str, Any]) -> tuple[RunLayout, Manifest, Path]:
    """A written plan, and a session Cosmic Ray has just initialized from it."""
    layout, manifest, project = _planned(tmp_path, *mutants)
    engine.init_session(layout, project)
    return layout, manifest, project


def _jobs_by_operator(layout: RunLayout) -> dict[str, list[str]]:
    """Every job in the session, grouped by the operator that made it."""
    grouped: dict[str, list[str]] = {}
    with engine.open_session(layout.session) as session:
        for item in session.work_items:
            grouped.setdefault(item.mutations[0].operator_name, []).append(item.job_id)
    return grouped


def _outcomes(layout: RunLayout) -> dict[str, str]:
    with engine.open_session(layout.session) as session:
        return {job_id: result.worker_outcome.value for job_id, result in session.results}


def test_initializing_a_session_creates_jobs_for_this_packs_operator(tmp_path: Path) -> None:
    layout, _, _ = _initialized(tmp_path, _mutant("first"))

    grouped = _jobs_by_operator(layout)

    assert len(grouped[FULL_OPERATOR_NAME]) == 1
    # The built-in operators are instantiated regardless of the configuration,
    # which is the flood the filter exists to stop.
    assert sum(len(jobs) for name, jobs in grouped.items() if name != FULL_OPERATOR_NAME) > 0


def test_filtering_leaves_exactly_one_job_per_mutant(tmp_path: Path) -> None:
    layout, manifest, _ = _initialized(
        tmp_path, _mutant("first"), _mutant("second", replacement="start > end")
    )

    jobs = engine.filter_jobs(layout.session, manifest)

    assert sorted(jobs) == ["first", "second"]
    assert len(set(jobs.values())) == 2


def test_filtering_skips_every_job_that_is_not_a_manifest_mutant(tmp_path: Path) -> None:
    layout, manifest, _ = _initialized(tmp_path, _mutant("first"))

    jobs = engine.filter_jobs(layout.session, manifest)

    outcomes = _outcomes(layout)
    kept = set(jobs.values())
    for operator, group in _jobs_by_operator(layout).items():
        for job_id in group:
            if job_id in kept:
                assert job_id not in outcomes, "a mutant's own job must stay pending"
            else:
                assert outcomes[job_id] == "skipped", f"a {operator} job was left to run"


def test_a_mutant_that_also_matches_another_file_keeps_only_its_own_job(
    tmp_path: Path,
) -> None:
    # The operator never learns which file it is looking at, so two files holding
    # the same code at the same position both match. Without the file check the
    # duplicates would make the one-job-per-mutant count fail on a manifest that
    # is perfectly valid.
    layout, manifest, _ = _initialized(
        tmp_path, _mutant("first"), _mutant("twin", file=TWIN, replacement="start > end")
    )
    before = _jobs_by_operator(layout)[FULL_OPERATOR_NAME]

    jobs = engine.filter_jobs(layout.session, manifest)

    assert len(before) == 4, "each mutant should match in both identical files"
    assert sorted(jobs) == ["first", "twin"]
    with engine.open_session(layout.session) as session:
        kept = {
            item.job_id: item.mutations[0].module_path.as_posix()
            for item in session.work_items
            if item.job_id in set(jobs.values())
        }
    assert kept[jobs["first"]] == TARGET
    assert kept[jobs["twin"]] == TWIN


def test_a_mutant_with_no_job_of_its_own_is_an_adapter_failure(tmp_path: Path) -> None:
    # Language validation has already established that the span matches a node,
    # so a mutant with no job means the adapter lost it — never a manifest
    # problem, and never something to pass over quietly.
    layout, manifest, _ = _initialized(
        tmp_path, _mutant("first"), _mutant("ghost", original="start > end")
    )

    with pytest.raises(PackFailure) as raised:
        engine.filter_jobs(layout.session, manifest)

    assert (raised.value.stage.value, raised.value.code) == ("plan", "mutant_job_mismatch")
    assert "ghost" in raised.value.message


def test_a_session_cosmic_ray_refuses_to_build_is_reported_as_a_plan_failure(
    tmp_path: Path,
) -> None:
    # A target that disappears between planning and initialization: Cosmic Ray
    # fails, and its output has to reach the run's logs rather than nowhere.
    layout, _, project = _planned(tmp_path, _mutant("first"))
    (project / TARGET).unlink()

    with pytest.raises(PackFailure) as raised:
        engine.init_session(layout, project)

    assert (raised.value.stage.value, raised.value.code) == ("plan", "init_failed")
    assert (layout.logs / "init.txt").exists()
