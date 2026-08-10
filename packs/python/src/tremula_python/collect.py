"""Rebuild what a run observed, from the run directory and nothing else.

`collect` is given one path. Everything else it rediscovers there — the manifest
copy, the table of hashes each file should have had, the diffs rendered while the
sources were still intact, the backend's own session — which is what makes it
usable after an interrupted run, and what makes it idempotent: run it twice and
the second `results.json` is byte for byte the first.

Nothing here judges. A verdict is the core's to decide, and the whole job of this
module is to hand it signals that mean the same thing in every language: which
attempt reached which point, and what the test runner reported. The backend's own
vocabulary is preserved untranslated under `backend_raw`, for cross-checking only.

Two things are easy to get wrong and are guarded deliberately.

*Which job belongs to which mutant.* The operator matches by position and text
with no knowledge of files, so a mutant appears once for its own file and once for
every other file that happens to hold the same code at the same place; the filter
marked those duplicates skipped. A job is a mutant's own only when this pack's
operator made it, its arguments name that mutant, and it is about that mutant's
file — and each mutant produces exactly one entry, in manifest order, so a
duplicate can never overwrite a real result.

*What is not known.* An attempt with no marker gets no runner block rather than a
zeroed one, and `finished_at` is null throughout: the session records no clock
time, and inventing one would be a lie that also broke idempotence.
"""

from dataclasses import dataclass
from pathlib import Path

from cosmic_ray.work_item import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    WorkerOutcome,
    WorkResult,
)

from tremula_python import capabilities, engine, marker, plan_session, refusals
from tremula_python.contracts import (
    CONTRACT_VERSION,
    ExecutionStatus,
    Location,
    Manifest,
    Mutant,
    ResultEntry,
    Results,
    Stage,
)
from tremula_python.cr_operator import FULL_OPERATOR_NAME
from tremula_python.engine import MUTANT_ID_ARGUMENT, Session
from tremula_python.errors import PackFailure
from tremula_python.plan_session import ExpectedHashes
from tremula_python.pytest_runner import Marker
from tremula_python.refusals import Refusal
from tremula_python.run_layout import RunLayout, write_atomically

TIMEOUT_OUTPUT = "timeout"
"""What Cosmic Ray stores as a job's output when its own time limit fired.

The whole output, not a line of it: the test process is gone by then, and with it
the marker. An attempt that says only this is a timeout with no runner signals.
"""

INCOMPLETE_RUN_DIR = "incomplete_run_dir"
"""Code for a run directory that cannot be read as a run."""


@dataclass(frozen=True)
class _Job:
    """The one backend job that belongs to a mutant."""

    location: Location
    result: WorkResult | None
    reported: Marker | None


@dataclass(frozen=True)
class _Reading:
    """What an attempt amounts to, and — when it needs saying — why."""

    status: ExecutionStatus
    reason: str | None = None


def collect(run_dir: Path) -> Results:
    """Read a run's results out of its directory.

    Args:
        run_dir: The run directory, whose name is the run identifier.

    Returns:
        The results document, which is also written into the directory.

    Raises:
        PackFailure: The directory is missing something a run leaves behind.
    """
    layout = RunLayout.at(run_dir)
    _require_a_readable_run(layout)
    manifest = Manifest.model_validate_json(layout.manifest.read_bytes())
    hashes = plan_session.read_expected_hashes(layout.expected_hashes)
    diffs = plan_session.read_diffs(layout.diffs)
    refused = refusals.read(layout)
    with engine.open_session(layout.session) as session:
        jobs = _jobs_by_mutant(session, manifest)
    entries: list[ResultEntry] = []
    for mutant in manifest.mutants:
        job = jobs.get(mutant.id)
        refusal = refused.get(mutant.id)
        reading = _read(job, mutant, hashes, refusal)
        entries.append(_entry(mutant, job, reading, diffs.get(mutant.id), refusal))
        _keep_the_output(layout, mutant, job, reading)
    results = Results(
        schema_version=CONTRACT_VERSION,
        run_id=layout.run_id,
        pack=capabilities.pack_info(),
        entries=entries,
    )
    write_atomically(layout.results, results.model_dump_json())
    return results


def _require_a_readable_run(layout: RunLayout) -> None:
    """Refuse a directory that is not a run this pack planned.

    Raises:
        PackFailure: The manifest copy, the hash table, the diffs, or the session
            is missing.
    """
    missing = [
        path.name
        for path in (layout.manifest, layout.expected_hashes, layout.diffs, layout.session)
        if not path.is_file()
    ]
    if not missing:
        return
    raise PackFailure(
        Stage.COLLECT,
        INCOMPLETE_RUN_DIR,
        f"{layout.directory} is missing {', '.join(missing)}, which a run writes before it "
        "starts executing; collect the run directory a `run` produced, or start a new run",
    )


def _jobs_by_mutant(session: Session, manifest: Manifest) -> dict[str, _Job]:
    """Pick each mutant's own job out of the session.

    A job left skipped is only accepted when the mutant has nothing else, so a
    filtered duplicate cannot displace the attempt that really ran.
    """
    files = {mutant.id: mutant.file for mutant in manifest.mutants}
    results = dict(session.results)
    chosen: dict[str, _Job] = {}
    for item in session.work_items:
        mutation = item.mutations[0]
        if mutation.operator_name != FULL_OPERATOR_NAME:
            continue
        identifier: object = mutation.operator_args.get(MUTANT_ID_ARGUMENT)
        if not isinstance(identifier, str):
            continue
        if files.get(identifier) != mutation.module_path.as_posix():
            continue
        previous = chosen.get(identifier)
        if previous is not None and not _was_filtered_out(previous.result):
            continue
        result = results.get(item.job_id)
        chosen[identifier] = _Job(
            location=Location(line=mutation.start_pos[0], column=mutation.start_pos[1]),
            result=result,
            reported=marker.parse_last(result.output) if result and result.output else None,
        )
    return chosen


def _was_filtered_out(result: WorkResult | None) -> bool:
    return result is not None and result.worker_outcome == WorkerOutcome.SKIPPED


def _read(
    job: _Job | None, mutant: Mutant, hashes: ExpectedHashes, refusal: Refusal | None
) -> _Reading:
    """Translate one attempt into a neutral status, first matching rule winning.

    The order is the contract. A refusal outranks everything, because it happened
    before the session did: the run decided this mutation could not be applied, and
    whatever the session says about it — nothing, or a job left skipped because
    nobody could tell which of two matches it was — is a consequence of that
    decision rather than a second opinion about it.

    A timeout then outranks both the missing marker and the hash comparison,
    because the runner writes the hashes before the suite starts: a run it had to
    kill still carries them, and calling that a hash problem would hide the timeout
    behind an unrelated diagnosis.
    """
    if refusal is not None:
        return _Reading(ExecutionStatus.NOT_APPLIED, refusal.message)
    if job is None:
        return _Reading(
            ExecutionStatus.NOT_APPLIED,
            "the backend produced no job for this mutant",
        )
    if job.result is None:
        return _Reading(ExecutionStatus.NOT_RUN)
    outcome = job.result.worker_outcome
    if outcome == WorkerOutcome.SKIPPED:
        return _Reading(ExecutionStatus.SKIPPED)
    if outcome == WorkerOutcome.NO_TEST:
        return _Reading(
            ExecutionStatus.NOT_APPLIED,
            "the backend reported that no mutation could be applied",
        )
    if outcome in (WorkerOutcome.EXCEPTION, WorkerOutcome.ABNORMAL):
        return _Reading(
            ExecutionStatus.BACKEND_ERROR,
            f"the backend ended this job as `{outcome.value}`",
        )
    if job.result.output == TIMEOUT_OUTPUT or (job.reported and job.reported["timed_out"]):
        return _Reading(ExecutionStatus.TIMEOUT)
    if job.reported is None:
        return _Reading(
            ExecutionStatus.BACKEND_ERROR,
            "the test runner printed no result marker, so nothing is known about the suite",
        )
    problem = _hash_problem(job.reported, mutant, hashes)
    if problem is not None:
        return _Reading(ExecutionStatus.BACKEND_ERROR, problem)
    return _Reading(ExecutionStatus.COMPLETED)


def _hash_problem(reported: Marker, mutant: Mutant, hashes: ExpectedHashes) -> str | None:
    """Why the files on disk were not what this mutant should have made of them.

    The runner hashes every target file, so this catches three different failures
    with one comparison: the mutation not being applied, a different mutation
    being applied, and some other target file having changed underneath the run.
    """
    observed = reported["target_hashes"]
    if not observed:
        return "the test runner reported no file hashes, so the mutation cannot be confirmed"
    expected = hashes.mutated.get(mutant.id)
    if expected is None:
        return "the run directory carries no expected hash for this mutant"
    if observed.get(mutant.file) != expected:
        return (
            f"{mutant.file} hashed {observed.get(mutant.file)} while the suite ran, "
            f"not the {expected} this mutation makes of it"
        )
    for file, digest in sorted(hashes.base.items()):
        if file == mutant.file:
            continue
        if observed.get(file) != digest:
            return (
                f"{file} hashed {observed.get(file)} while the suite ran, but no mutation "
                f"was meant to touch it (it should have stayed {digest})"
            )
    return None


def _entry(
    mutant: Mutant,
    job: _Job | None,
    reading: _Reading,
    diff: str | None,
    refusal: Refusal | None,
) -> ResultEntry:
    """One mutant's entry.

    Every field is either observed or absent. The diff is the one the plan
    rendered while the sources were still intact — a diff taken now could describe
    a file an interrupted run left mutated. `finished_at` is always absent: the
    session records no time of its own, and `truncated` is always false because
    nothing here shortens what it captured.

    A refused mutant reports no runner and no location, because it was never given
    either.
    """
    refused = refusal is not None
    return ResultEntry(
        mutant_id=mutant.id,
        execution_status=reading.status,
        location=job.location if job is not None and not refused else None,
        runner=(
            marker.runner_result_of(job.reported)
            if job is not None and job.reported is not None and not refused
            else None
        ),
        diff=diff,
        truncated=False,
        finished_at=None,
        backend_raw=_backend_raw(job, refusal),
    )


def _backend_raw(job: _Job | None, refusal: Refusal | None) -> dict[str, object]:
    """The backend's own account of the job, preserved and never judged.

    A refusal is not the backend's account of anything — it is the run's, and it
    is why there is no account to keep — but this is the one place the results
    contract leaves open for it, and a reader that finds a mutation never applied
    needs to be told why in the document rather than sent looking for a log.
    """
    raw: dict[str, object] = {}
    if refusal is not None:
        raw["refusal"] = {"code": refusal.code, "message": refusal.message}
    if job is None or job.result is None:
        return raw
    result = job.result
    raw.update(
        {
            "worker_outcome": result.worker_outcome.value,
            "test_outcome": result.test_outcome.value if result.test_outcome else None,
            "output": result.output,
            "diff": result.diff,
            "pytest_exit": job.reported["pytest_exit"] if job.reported else None,
        }
    )
    return raw


def _keep_the_output(
    layout: RunLayout, mutant: Mutant, job: _Job | None, reading: _Reading
) -> None:
    """Write what the suite printed for this mutant, and why it was read that way.

    The results document has nowhere to record *why* an attempt was a backend
    error, and that reason is the first thing a reader needs.
    """
    if job is None or job.result is None:
        if reading.reason is None:
            return
        write_atomically(layout.logs / f"{mutant.id}.txt", f"tremula-pack: {reading.reason}\n")
        return
    output = job.result.output or ""
    if reading.reason is not None:
        output = f"{output.rstrip()}\ntremula-pack: {reading.reason}\n".lstrip()
    write_atomically(layout.logs / f"{mutant.id}.txt", output)
