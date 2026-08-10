"""Drive Cosmic Ray, and cut the session it builds down to the manifest.

Cosmic Ray is invoked as the command it is, not imported: `init` and `exec` are
its supported entry points, and the pack has no business reaching past them. Two
details of how that invocation is set up matter.

*The executable is chosen, not searched for.* It is taken from the directory of
the running interpreter, so the Cosmic Ray that runs is the one preflight checked
and the one this pack's operator is registered in — never whichever copy happens
to be first on `PATH`.

*The working directory is the project root.* Every relative path in the
configuration is resolved against it, and the diff headers Cosmic Ray writes carry
those paths verbatim.

Filtering is the other half. An initialized session holds far more than the
manifest asked for: Cosmic Ray instantiates every built-in operator whether the
configuration mentions it or not, and this pack's operator matches by position and
text with no idea which file it is looking at, so two identical files both match.
Marking everything else as skipped — the same way Cosmic Ray's own filter tools do
— leaves exactly one job per mutant, and that is what makes a *missing* job mean
something: the language checks already established that every span matches a node,
so a mutant with no job is the adapter's fault. It is named as such and left out,
rather than ending a run whose other mutants are perfectly good.
"""

import shlex
import subprocess
import sys
from collections.abc import Generator, Iterable
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, cast

from cosmic_ray.work_db import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    WorkDB,
    use_db,  # pyright: ignore[reportUnknownVariableType]
)
from cosmic_ray.work_item import (  # pyright: ignore[reportMissingTypeStubs]
    WorkerOutcome,
    WorkItem,
    WorkResult,
)

from tremula_python.contracts import Manifest, Stage
from tremula_python.cr_operator import FULL_OPERATOR_NAME
from tremula_python.errors import PackFailure
from tremula_python.refusals import Refusal, Refused
from tremula_python.run_layout import RunLayout, write_atomically

MUTANT_ID_ARGUMENT = "mutant_id"
"""The operator argument that carries the manifest identifier through the work-db."""

MUTANT_JOB_MISMATCH = "mutant_job_mismatch"
"""Code for a mutant the backend made the wrong number of jobs for."""


class Session(Protocol):
    """The work-db surface this pack uses, in the types it really has.

    Cosmic Ray ships no `py.typed`, so this protocol is the typed boundary: the
    database is opened once, in `open_session`, and everything downstream works
    against these three members.
    """

    @property
    def work_items(self) -> tuple[WorkItem, ...]: ...

    @property
    def results(self) -> Iterable[tuple[str, WorkResult]]: ...

    def set_result(self, job_id: str, result: WorkResult) -> None: ...


@contextmanager
def open_session(session_path: Path) -> Generator[Session, None, None]:
    """Open an existing session for reading and marking.

    Opened in `open` mode on purpose. The default mode creates the file when it is
    missing, which would turn "this run has no session" into an empty session and
    report every mutant as never having run.

    Raises:
        FileNotFoundError: There is no session at that path.
    """
    with use_db(session_path, WorkDB.Mode.open) as database:  # pyright: ignore[reportUnknownVariableType, reportUnknownArgumentType]
        yield cast("Session", database)


def init_session(layout: RunLayout, project_root: Path) -> None:
    """Build a fresh session from the generated configuration.

    Raises:
        PackFailure: Cosmic Ray could not build the session.
    """
    _run(
        ["init", str(layout.config), str(layout.session)],
        project_root,
        layout.logs / "init.txt",
        Stage.PLAN,
        "init_failed",
        "Cosmic Ray could not build a session from the generated configuration",
    )


def execute(layout: RunLayout, project_root: Path) -> None:
    """Run every pending job in the session.

    Raises:
        PackFailure: Cosmic Ray stopped before it finished. The session survives,
            so a later `collect` still reports what did run.
    """
    _run(
        ["exec", str(layout.config), str(layout.session)],
        project_root,
        layout.logs / "exec.txt",
        Stage.EXECUTE,
        "exec_failed",
        "Cosmic Ray stopped before it finished running the session",
    )


@dataclass(frozen=True)
class Filtered:
    """What the filter left behind: one job each, and the mutants it could not."""

    jobs: dict[str, str]
    """The job left pending for each mutant that has exactly one, by identifier."""

    refused: Refused
    """Why each of the others was left out instead."""


def filter_jobs(session_path: Path, manifest: Manifest) -> Filtered:
    """Leave one pending job per mutant, mark everything else skipped, and say what was left out.

    A mutant with no job of its own means the adapter lost it — the span was
    checked against the parser before the session was built — and a mutant with
    more than one means two nodes answer to the same description, so neither of
    them is on its own the mutation the manifest asked for. Both are refused, and
    the second has every one of its jobs marked skipped: picking one of two
    indistinguishable matches would report a mutation nobody described.

    Refusing rather than raising is what keeps one such mutant from costing the
    rest of the batch its verdicts.
    """
    kept: dict[str, list[str]] = {mutant.id: [] for mutant in manifest.mutants}
    wanted = {mutant.id: mutant.file for mutant in manifest.mutants}
    with open_session(session_path) as session:
        for item in session.work_items:
            mutant_id = _mutant_of(item, wanted)
            if mutant_id is None:
                session.set_result(item.job_id, WorkResult(worker_outcome=WorkerOutcome.SKIPPED))
                continue
            kept[mutant_id].append(item.job_id)
        refused = {
            mutant_id: Refusal(MUTANT_JOB_MISMATCH, _why_the_count_is_wrong(mutant_id, jobs))
            for mutant_id, jobs in sorted(kept.items())
            if len(jobs) != 1
        }
        for mutant_id in refused:
            for job_id in kept[mutant_id]:
                session.set_result(job_id, WorkResult(worker_outcome=WorkerOutcome.SKIPPED))
    return Filtered(
        jobs={
            mutant_id: jobs[0]
            for mutant_id, jobs in kept.items()
            if mutant_id not in refused
        },
        refused=refused,
    )


def _why_the_count_is_wrong(mutant_id: str, jobs: list[str]) -> str:
    """Why a mutant the backend counted wrongly is left out rather than run."""
    if not jobs:
        return (
            f"mutant {mutant_id}: the backend produced no job for it, though its span was "
            "checked against the parser before the session was built; this is a defect in "
            "the adapter rather than in the manifest"
        )
    return (
        f"mutant {mutant_id}: the backend produced {len(jobs)} jobs for this one mutant, so no "
        "single one of them is the mutation the manifest describes; every one of them was "
        "left unrun rather than one of them chosen"
    )


def _mutant_of(item: WorkItem, wanted: dict[str, str]) -> str | None:
    """The mutant this job belongs to, or None if it belongs to no mutant.

    A job is a mutant's own when this pack's operator made it, its arguments name
    that mutant, and it is about the file the mutant is in. The last condition is
    not redundant: the operator matches identical code in any file, so a mutant
    that appears twice in a session appears once for its own file and once for
    somebody else's.
    """
    mutation = item.mutations[0]
    if mutation.operator_name != FULL_OPERATOR_NAME:
        return None
    identifier: object = mutation.operator_args.get(MUTANT_ID_ARGUMENT)
    if not isinstance(identifier, str) or identifier not in wanted:
        return None
    if mutation.module_path.as_posix() != wanted[identifier]:
        return None
    return identifier


def _run(
    arguments: list[str],
    project_root: Path,
    log: Path,
    stage: Stage,
    code: str,
    summary: str,
) -> None:
    """Run one Cosmic Ray command, keeping its output, and fail loudly if it did.

    Raises:
        PackFailure: The command exited non-zero.
    """
    executable = Path(sys.executable).parent / "cosmic-ray"
    command = [str(executable), *arguments]
    completed = subprocess.run(
        command,
        cwd=project_root,
        capture_output=True,
        text=True,
        check=False,
    )
    # Quoted, because the first line of the log exists to be run again: a run
    # directory with a space in its path would otherwise be pasted as two words.
    write_atomically(log, f"$ {shlex.join(command)}\n{completed.stdout}{completed.stderr}")
    if completed.returncode == 0:
        return
    raise PackFailure(
        stage,
        code,
        f"{summary}: it exited {completed.returncode}"
        + _last_words(completed.stderr or completed.stdout)
        + f"; the full output is in {log}",
    )


def _last_words(output: str) -> str:
    """The final line of a command's output, for a message that has to be short."""
    lines = [line for line in output.splitlines() if line.strip()]
    if not lines:
        return ""
    return f" saying `{lines[-1].strip()}`"
