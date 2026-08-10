"""Running the one input that would tell two versions of a function apart.

A survivor arrives with a claim about it and, when the claim is worth anything,
with one call said to separate the mutation from the original. This module runs
that call against both versions and reports what each did. It is the only part of
triage whose answer is an observation rather than something a model said.

The shape of the work, and why each part of it is the way it is:

* **Both versions come out of the same bytes the caller was given.** The original
  side is the file as it stands under the root this was pointed at; the mutant side
  is those bytes with the span replaced, spliced by the one splice in the pack, so
  it is the same file a run would have applied.
* **Each version runs three times, in a subprocess of its own each time.** A
  version that does not agree with itself cannot be compared with anything, and
  loading two copies of one module into one process is a way to make them disagree
  for reasons that have nothing to do with the mutation.
* **Every subprocess gets a hash seed and almost no environment.** `PYTHONHASHSEED`
  is fixed, which is why the children are not started with `-I`: that flag implies
  ignoring the environment, and the seed is part of the environment. What is passed
  instead is a short list of variables, so nothing the caller happened to export
  reaches the code being measured.
* **A subprocess that will not finish is killed with its whole process group.** A
  witness naming an input that never returns is a real answer — that the call could
  not be observed — and it must not be a hung pack.

**No new isolation is claimed.** The mutated function's own body executes, which is
exactly what `run` does when it applies a mutant and calls the suite: the threat
model is that one and no weaker. What is not evaluated is the witness — its
arguments are read by the list of syntax `witness` keeps, so nothing a model wrote
is ever executed as code.

**POSIX only**, for the same reason `pytest_runner` is: process-group isolation
uses `start_new_session` and `os.killpg`.
"""

import ast
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast

from tremula_python import probe_child, target_file
from tremula_python.contracts import (
    CONTRACT_VERSION,
    Ending,
    Observation,
    ProbeOutcome,
    ProbeReport,
    Stage,
    Undecided,
)
from tremula_python.cr_operator import file_with_the_mutation
from tremula_python.errors import PackFailure
from tremula_python.witness import function_named

RUNS_PER_SIDE = 3
"""How many times each version is run. See the module docstring."""

DEFAULT_TIMEOUT_SECONDS = 10.0
"""How long one run of one version may take before its process group is killed.

Generous for a single call of a single function, and short enough that a witness
naming an input that never returns costs a minute rather than a run.
"""

_KEPT_ENVIRONMENT = ("PATH", "HOME", "TMPDIR", "TEMP", "TMP", "LANG", "LC_ALL")
"""The only variables a run inherits. Everything else is left behind on purpose."""

_SKIPPED_DIRECTORIES = frozenset({"__pycache__", ".git", ".tremula"})
"""Directories not worth copying into either side of a probe."""

_Function = ast.FunctionDef | ast.AsyncFunctionDef
"""The two ways a file spells a function. A lambda is an expression, not one."""


@dataclass(frozen=True)
class Witness:
    """What to run: a file, a span to replace in it, and one call to make."""

    file: str
    start_byte: int
    end_byte: int
    replacement: str
    call: str


def probe(
    project_root: Path, witness: Witness, timeout: float = DEFAULT_TIMEOUT_SECONDS
) -> ProbeReport:
    """Run `witness` against both versions of its function and say what happened.

    Never reports a difference it did not observe. Every way of failing to observe
    one — a call this will not evaluate, a name that is not a module-level function,
    a version that disagrees with itself, a value that cannot be compared across
    processes, a run that had to be killed — comes back as `undecided` with the
    reason, because each of those is a fact about the witness or the value and none
    of them is a fact about the mutation.

    Raises:
        PackFailure: The file is not one of this project's own readable Python
            sources, the span does not fall inside it, or a version of it could not
            be imported at all.
    """
    root = target_file.canonical_root(project_root, Stage.PROBE)
    source = target_file.read(root, witness.file, Stage.PROBE)
    text = target_file.decoded(source, witness.file, Stage.PROBE)
    _within(source, witness)
    refused = _refusal(text, witness)
    if refused is not None:
        return _undecided(witness, refused)
    mutated = file_with_the_mutation(
        source,
        witness.start_byte,
        witness.end_byte,
        source[witness.start_byte : witness.end_byte].decode("utf-8"),
        witness.replacement,
    )
    with tempfile.TemporaryDirectory(prefix="tremula-probe-") as workspace:
        sides = {
            "original": _side(Path(workspace) / "original", root, witness.file, source),
            "mutant": _side(Path(workspace) / "mutant", root, witness.file, mutated),
        }
        observed = {
            name: _agreed(_runs(module, search, witness.call, timeout))
            for name, (module, search) in sides.items()
        }
    return _compared(witness, observed["original"], observed["mutant"])


def module_level_functions(module: ast.Module) -> set[str]:
    """The names a module defines as functions of its own, methods excluded."""
    return {
        statement.name
        for statement in module.body
        if isinstance(statement, _Function)
    }


def _refusal(text: str, witness: Witness) -> Undecided | None:
    """Why this witness cannot be run at all, or None if it can be."""
    named = function_named(witness.call)
    if named is None:
        return Undecided.UNSAFE_WITNESS
    module = target_file.parsed(text, witness.file, Stage.PROBE)
    if named in module_level_functions(module):
        return None
    if named in _method_names(module):
        return Undecided.METHOD
    return Undecided.NO_SUCH_FUNCTION


def _method_names(module: ast.Module) -> set[str]:
    """Every function a class in the module defines, at any depth inside it."""
    found: set[str] = set()
    for statement in module.body:
        if isinstance(statement, ast.ClassDef):
            for node in ast.walk(statement):
                if isinstance(node, _Function):
                    found.add(node.name)
    return found


def _within(source: bytes, witness: Witness) -> None:
    """Check the span is a range of bytes this file has.

    Raises:
        PackFailure: The span is empty, inverted, or reaches past the file.
    """
    if 0 <= witness.start_byte < witness.end_byte <= len(source):
        return
    raise PackFailure(
        Stage.PROBE,
        "span_outside_file",
        f"bytes {witness.start_byte}..{witness.end_byte} are not a range of "
        f"`{witness.file}`, which is {len(source)} bytes long",
    )


def _side(root: Path, from_root: Path, relative: str, bytes_of_it: bytes) -> tuple[Path, list[str]]:
    """One version of the project, and where its imports are looked up.

    A copy rather than the tree itself, so that neither version can be affected by
    the other and neither leaves anything behind in the project. The file's own
    directory is on the search path as well as the root, because a module that
    imports a sibling by its bare name is looked up from where it lives.
    """
    _copy_tree(from_root, root)
    target = root / Path(relative)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(bytes_of_it)
    return target, [str(root), str(target.parent)]


def _copy_tree(from_root: Path, into: Path) -> None:
    """Copy every ordinary file of a tree, and nothing else.

    A link is not copied and not followed. Following one would read a file outside
    the tree the caller pointed at, which is the same refusal the target file itself
    is held to, and copying one as a link would leave it pointing at whatever it
    pointed at before.
    """
    into.mkdir(parents=True, exist_ok=True)
    for entry in from_root.iterdir():
        if entry.is_symlink() or entry.name in _SKIPPED_DIRECTORIES:
            continue
        if entry.is_dir():
            _copy_tree(entry, into / entry.name)
        elif entry.is_file():
            shutil.copyfile(entry, into / entry.name)


def _runs(module: Path, search_path: list[str], call: str, timeout: float) -> list[dict[str, Any]]:
    """Run one version as many times as `RUNS_PER_SIDE` says, each on its own."""
    with tempfile.TemporaryDirectory(prefix="tremula-probe-job-") as directory:
        job = Path(directory) / "job.json"
        job.write_text(
            json.dumps({"module": str(module), "search_path": search_path, "call": call}),
            encoding="utf-8",
        )
        return [_one_run(job, timeout) for _ in range(RUNS_PER_SIDE)]


def _one_run(job: Path, timeout: float) -> dict[str, Any]:
    """Start one child, read its one line, and never leave it behind.

    Raises:
        PackFailure: The child could not be started, or said nothing this can read.
    """
    command = [sys.executable, "-s", "-B", "-m", probe_child.__name__, str(job)]
    try:
        process = subprocess.Popen(  # noqa: S603 - the interpreter running this pack
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            encoding="utf-8",
            errors="replace",
            env=_environment(),
            start_new_session=True,
        )
    except OSError as error:
        raise PackFailure(
            Stage.PROBE,
            "probe_not_started",
            f"a probe could not be started with `{sys.executable}`: {error}",
        ) from error
    try:
        said, _ = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        _kill_group(process.pid)
        process.communicate()
        return {"kind": "timed_out"}
    return _read(said, process.returncode)


def _read(said: str, status: int | None) -> dict[str, Any]:
    """The child's account, or the failure that it gave none.

    Raises:
        PackFailure: The child printed nothing this can read as an account.
    """
    lines = [line for line in said.splitlines() if line.strip()]
    if lines:
        try:
            account: object = json.loads(lines[-1])
        except json.JSONDecodeError:
            account = None
        if isinstance(account, dict):
            return {str(key): value for key, value in cast("dict[str, Any]", account).items()}
    raise PackFailure(
        Stage.PROBE,
        "probe_said_nothing",
        f"a probe exited with status {status} without saying what it observed; "
        f"it last printed {lines[-1] if lines else '(nothing)'!r}",
    )


def _environment() -> dict[str, str]:
    """A short, fixed environment with the hash seed pinned."""
    kept = {name: os.environ[name] for name in _KEPT_ENVIRONMENT if name in os.environ}
    kept["PYTHONHASHSEED"] = "0"
    kept["PYTHONDONTWRITEBYTECODE"] = "1"
    return kept


def _kill_group(pid: int) -> None:
    """Kill the child and anything it started, so nothing outlives the limit."""
    try:
        os.killpg(os.getpgid(pid), signal.SIGKILL)
    except OSError:
        return


@dataclass(frozen=True)
class _Side:
    """What one version did, once its runs have been made to agree — or why not."""

    account: dict[str, Any] | None
    undecided: Undecided | None


def _agreed(runs: list[dict[str, Any]]) -> _Side:
    """One account of a version, or the reason there is not one.

    Raises:
        PackFailure: The version could not be imported, which is not a fact about
            the mutation: a file neither version can load is a file nothing can be
            measured on.
    """
    for run in runs:
        if run.get("kind") == "import_failed":
            raise PackFailure(
                Stage.PROBE,
                "probe_import_failed",
                f"a version of the file could not be imported: {run.get('detail')}",
            )
    if any(run.get("kind") == "timed_out" for run in runs):
        return _Side(None, Undecided.TIMED_OUT)
    if any(run.get("kind") in {"not_literal", "not_a_call"} for run in runs):
        return _Side(None, Undecided.UNSAFE_WITNESS)
    if any(run.get("kind") == "no_such_function" for run in runs):
        return _Side(None, Undecided.NO_SUCH_FUNCTION)
    first = runs[0]
    if any(run.get("fingerprint") != first.get("fingerprint") for run in runs[1:]):
        return _Side(None, Undecided.NONDETERMINISTIC)
    if first.get("fingerprint") is None:
        return _Side(None, Undecided.INCOMPARABLE)
    return _Side(first, None)


def _compared(witness: Witness, original: _Side, mutant: _Side) -> ProbeReport:
    """The report: what each version did, and whether that was the same thing."""
    undecided = original.undecided or mutant.undecided
    outcome = ProbeOutcome.UNDECIDED
    if undecided is None:
        same = _fingerprint_of(original) == _fingerprint_of(mutant)
        outcome = ProbeOutcome.INDISTINGUISHABLE if same else ProbeOutcome.DIFFERS
    return ProbeReport(
        schema_version=CONTRACT_VERSION,
        file=witness.file,
        call=witness.call,
        outcome=outcome,
        undecided=undecided,
        original=_observation(original),
        mutant=_observation(mutant),
        runs_per_side=RUNS_PER_SIDE,
    )


def _fingerprint_of(side: _Side) -> str | None:
    """The one string a version is compared by."""
    return None if side.account is None else str(side.account.get("fingerprint"))


def _observation(side: _Side) -> Observation | None:
    """A version's account as the contract carries it, or nothing to carry."""
    if side.account is None:
        return None
    account = side.account
    return Observation(
        ended=Ending(str(account.get("ended"))),
        value=_string(account.get("value")),
        type_name=_string(account.get("type_name")),
        message=_string(account.get("message")),
        stdout=_string(account.get("stdout")) or "",
    )


def _string(value: object) -> str | None:
    """A field of the child's account, when it really is text."""
    return value if isinstance(value, str) else None


def _undecided(witness: Witness, why: Undecided) -> ProbeReport:
    """A report about a witness nothing was run for, and why nothing was."""
    return ProbeReport(
        schema_version=CONTRACT_VERSION,
        file=witness.file,
        call=witness.call,
        outcome=ProbeOutcome.UNDECIDED,
        undecided=why,
        original=None,
        mutant=None,
        runs_per_side=RUNS_PER_SIDE,
    )


def as_document(report: ProbeReport) -> str:
    """The report as the one line of stdout its caller reads."""
    return report.model_dump_json(exclude_defaults=True)
