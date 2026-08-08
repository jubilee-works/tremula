"""Turn a manifest into a Cosmic Ray session, and into the means to check it.

The configuration is the whole of the hand-off: Cosmic Ray reads it at
initialization and again at execution, and everything the operator will ever know
about a mutant travels in it. Three details of that are load-bearing.

*Paths are relative to the project root.* Cosmic Ray resolves `module-path`
against its working directory and puts it verbatim into the diff headers it
produces, so an absolute path would make those diffs unusable elsewhere.

*The test command is a string Cosmic Ray splits with `shlex`.* It is assembled
with `shlex.join` rather than by concatenation, so a run directory with a space in
its path stays one argument.

*The runner's time limit is the shorter one.* Cosmic Ray's limit is a backstop:
when it fires, the test process is gone and with it the only structured account
of what happened. The runner is given the effective limit and Cosmic Ray a margin
above it, so the ordinary way a hanging suite ends is the one that can still be
reported.

Alongside the configuration this module computes the two things a run is read
against afterwards. First, what each target file must hash to while a given mutant
is applied, and what the others must keep — predicted with the operator's own
shaping helper, because a replacement and the text finally injected can differ by
a newline and a hand-rolled splice would condemn valid mutants. Second, each
mutant as a patch. Both are made now because now is the last moment the unmutated
sources are certainly on disk: a run that is interrupted can leave a target
mutated, and neither could be trusted after that.
"""

import difflib
import json
import shlex
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
from typing import cast

from cosmic_ray.config import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    deserialize_config,  # pyright: ignore[reportUnknownVariableType]
    serialize_config,  # pyright: ignore[reportUnknownVariableType]
)

from tremula_python.contracts import Baseline, Manifest, Mutant, Stage
from tremula_python.cr_operator import FULL_OPERATOR_NAME, shaped_like_the_span
from tremula_python.errors import PackFailure
from tremula_python.positions import byte_span_to_positions
from tremula_python.run_layout import RunLayout, write_atomically

BACKSTOP_MARGIN_SECONDS = 10.0
"""How much longer Cosmic Ray waits than the runner it is running.

Enough for the runner to kill a hung process group, collect what it printed, and
write its own summary — its drain window plus room for the cost of starting up.
"""

UNSERIALIZABLE_MUTANT = "unserializable_mutant"
"""Code for a mutant whose own text cannot be carried to the backend."""

CONTEXT_LINES = 3
"""How many unchanged lines a diff carries on each side of a change."""

NO_FINAL_NEWLINE = "\\ No newline at end of file"
"""What a patch says about a file whose last line has no newline."""

_SHOWN_CHARACTERS = 120
"""How much of a value a failure message quotes before trailing off."""

MINIMUM_TIMEOUT_SECONDS = 30.0
"""The shortest limit a derived timeout may be, however fast the baseline was."""

TIMEOUT_FACTOR = 10
"""How many times the baseline's own duration a mutated run is allowed to take.

A mutant can make a fast suite slow without hanging it, so the limit has to be
generous; the floor is there because a suite that finishes in milliseconds gives
a factor of ten nothing to work with.
"""


@dataclass(frozen=True)
class ExpectedHashes:
    """What the target files must hash to for a mutation to have been applied cleanly."""

    base: dict[str, str]
    """Each target file's hash with no mutation applied, by project-relative path."""

    mutated: dict[str, str]
    """The mutated file's hash for each mutant, by mutant identifier."""

    @property
    def document(self) -> dict[str, dict[str, str]]:
        """This table as the JSON document the run directory carries."""
        return {"base": self.base, "mutated": self.mutated}


@dataclass(frozen=True)
class SessionPlan:
    """Everything the run directory needs before Cosmic Ray is invoked."""

    config: str
    """The generated configuration, as TOML."""

    targets: tuple[str, ...]
    """The files the runner is asked to hash, by project-relative path."""

    hashes: ExpectedHashes
    """What those files must hash to."""

    diffs: dict[str, str]
    """Each mutant's change as a unified diff, by mutant identifier."""

    def write(self, layout: RunLayout, manifest_copy: str) -> None:
        """Write the plan into the run directory.

        Every file here is one a later `collect` rediscovers the run from, which
        is why the manifest is copied verbatim rather than re-serialized: it is
        the record of what was asked for.
        """
        write_atomically(layout.manifest, manifest_copy)
        write_atomically(layout.config, self.config)
        write_atomically(layout.targets, json.dumps(list(self.targets)))
        write_atomically(layout.expected_hashes, json.dumps(self.hashes.document))
        write_atomically(layout.diffs, json.dumps(self.diffs))


def effective_timeout(reference: Baseline, explicit: float | None) -> float:
    """How long one mutated run of the suite may take.

    An explicit limit is taken as given. Otherwise it is derived from the baseline,
    which is why the baseline runs before the session is planned.
    """
    if explicit is not None:
        return explicit
    return max(MINIMUM_TIMEOUT_SECONDS, reference.runner.duration_ms / 1000 * TIMEOUT_FACTOR)


def build_plan(
    manifest: Manifest,
    project_root: Path,
    layout: RunLayout,
    tests: Sequence[str],
    effective_timeout: float,
) -> SessionPlan:
    """Plan a session for `manifest`, without writing anything."""
    return SessionPlan(
        config=build_config(manifest, project_root, layout, tests, effective_timeout),
        targets=target_files(manifest),
        hashes=expected_hashes(manifest, project_root),
        diffs=build_diffs(manifest, project_root),
    )


def build_config(
    manifest: Manifest,
    project_root: Path,
    layout: RunLayout,
    tests: Sequence[str],
    effective_timeout: float,
) -> str:
    """The Cosmic Ray configuration that runs `manifest`, as TOML.

    Serialized through Cosmic Ray's own helper: a replacement is arbitrary source
    code, full of quotes and backslashes and newlines, and hand-written escaping
    is how such a value quietly turns into a different one. That helper is then
    held to its word, because it does not always keep it.

    Raises:
        PackFailure: Some mutant's text does not survive the session file.
    """
    parameterizations = [_parameterization(mutant, project_root) for mutant in manifest.mutants]
    text = serialize_config(
        {
            "module-path": list(target_files(manifest)),
            "timeout": effective_timeout + BACKSTOP_MARGIN_SECONDS,
            "test-command": test_command(layout, tests, effective_timeout),
            "excluded-modules": [],
            "distributor": {"name": "local"},
            "operators": {FULL_OPERATOR_NAME: parameterizations},
        }
    )
    _check_the_mutants_survived(text, parameterizations)
    return text


def _check_the_mutants_survived(text: str, parameterizations: list[dict[str, str | int]]) -> None:
    """Read the session file back and confirm every mutant is still itself.

    The TOML encoder Cosmic Ray uses does not round-trip every string: measured
    against toml 0.10.2, a value that *begins* with a run of quote characters comes
    back shortened, and nothing complains. A mutant that reached the backend
    altered would match nothing and be reported much later as a job that never
    appeared — a defect in the adapter, from the reader's point of view. Refusing
    it here says what actually happened, before anything runs.

    Raises:
        PackFailure: A value did not come back as it went in.
    """
    restored = cast("dict[str, object]", deserialize_config(text))
    operators = cast("dict[str, object]", restored.get("operators", {}))
    carried = cast("list[dict[str, object]]", operators.get(FULL_OPERATOR_NAME, []))
    if len(carried) != len(parameterizations):
        raise PackFailure(
            Stage.PLAN,
            UNSERIALIZABLE_MUTANT,
            f"the generated session file carries {len(carried)} of "
            f"{len(parameterizations)} mutants; some mutant's text does not survive being "
            "written as TOML",
        )
    for sent, back in zip(parameterizations, carried, strict=True):
        for field, value in sent.items():
            if back.get(field) == value:
                continue
            raise PackFailure(
                Stage.PLAN,
                UNSERIALIZABLE_MUTANT,
                f"mutant {sent['mutant_id']}: its `{field}` cannot be carried to the backend, "
                f"which reads its session as TOML — {_shortened(value)} comes back as "
                f"{_shortened(back.get(field))}. Rewrite the mutant so this text does not "
                "begin with a run of quote characters.",
            )


def _shortened(value: object) -> str:
    """A value as a message can afford to show it."""
    shown = repr(value)
    if len(shown) <= _SHOWN_CHARACTERS:
        return shown
    return f"{shown[:_SHOWN_CHARACTERS]}…"


def test_command(layout: RunLayout, tests: Sequence[str], effective_timeout: float) -> str:
    """The command Cosmic Ray runs for every job.

    It is fixed for the whole session — Cosmic Ray has nowhere to put per-mutant
    arguments — which is why the runner is told to hash *every* target file rather
    than the one mutant's, and why cross-contamination is detectable at all.
    """
    return shlex.join(
        [
            sys.executable,
            "-m",
            "tremula_python.pytest_runner",
            "--timeout",
            str(effective_timeout),
            "--hash-targets",
            str(layout.targets),
            "--pycache-prefix",
            str(layout.pycache),
            "--",
            *tests,
        ]
    )


def target_files(manifest: Manifest) -> tuple[str, ...]:
    """Every file the manifest touches, once each, in a stable order."""
    return tuple(sorted({mutant.file for mutant in manifest.mutants}))


def expected_hashes(manifest: Manifest, project_root: Path) -> ExpectedHashes:
    """Predict what the target files hash to, unmutated and per mutant."""
    sources = _read_sources(manifest, project_root)
    return ExpectedHashes(
        base={file: _digest(source) for file, source in sources.items()},
        mutated={
            mutant.id: _digest(_apply(mutant, sources[mutant.file]))
            for mutant in manifest.mutants
        },
    )


def build_diffs(manifest: Manifest, project_root: Path) -> dict[str, str]:
    """Render each mutant as a unified diff of whole lines, against the real file.

    Built here rather than when results are collected, because this is the last
    moment the unmutated file is certainly on disk: a run that is interrupted can
    leave a target mutated, and by then no diff taken from it would be true.

    Whole lines with surrounding context, so the result is a patch a reader can
    hand to `git apply` — a hunk holding only the span's own bytes describes the
    change accurately and applies nowhere, since a manifest span routinely starts
    and ends mid-line.
    """
    sources = _read_sources(manifest, project_root)
    return {
        mutant.id: _unified_diff(mutant, sources[mutant.file]) for mutant in manifest.mutants
    }


def read_diffs(path: Path) -> dict[str, str]:
    """Read written diffs back.

    Raises:
        ValueError: The document is not a table of identifiers to diffs.
    """
    document: object = json.loads(path.read_text(encoding="utf-8"))
    return _string_map(document, path, "diffs")


def read_expected_hashes(path: Path) -> ExpectedHashes:
    """Read a written hash table back.

    Raises:
        ValueError: The document is not a table of paths and identifiers to
            hashes.
    """
    document: object = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise ValueError(f"{path}: expected an object with `base` and `mutated`")
    fields = cast("dict[str, object]", document)
    return ExpectedHashes(
        base=_string_map(fields.get("base"), path, "base"),
        mutated=_string_map(fields.get("mutated"), path, "mutated"),
    )


def _parameterization(mutant: Mutant, project_root: Path) -> dict[str, str | int]:
    """One mutant as operator arguments: strings and integers only.

    Nothing richer survives the trip, which goes through TOML into the work-db and
    out again as JSON. The positions are the parser's — line numbers from one,
    columns counted in characters — because that is the only way the operator can
    recognize the node it is meant to change.
    """
    source_bytes = (project_root / mutant.file).read_bytes()
    start, end = byte_span_to_positions(source_bytes, mutant.span)
    return {
        "mutant_id": mutant.id,
        "start_line": start[0],
        "start_col": start[1],
        "end_line": end[0],
        "end_col": end[1],
        "original": mutant.original,
        "replacement": mutant.replacement,
    }


def _apply(mutant: Mutant, source_bytes: bytes) -> bytes:
    """The file as it will stand while `mutant` is applied.

    Exactly the contract: the bytes of the span, replaced. The one liberty is the
    operator's own shaping of the replacement, which strips a trailing newline the
    span does not include and restores one it does, so that both spellings of a
    replacement describe the same file. Nothing outside the span moves — a
    prediction that allowed it to would bless a mutation that changed more than
    the manifest asked for.
    """
    injected = shaped_like_the_span(mutant.replacement, mutant.original)
    return (
        source_bytes[: mutant.span.start_byte]
        + injected.encode("utf-8")
        + source_bytes[mutant.span.end_byte :]
    )


def _read_sources(manifest: Manifest, project_root: Path) -> dict[str, bytes]:
    """Every target file's current bytes, by project-relative path."""
    return {file: (project_root / file).read_bytes() for file in target_files(manifest)}


def _unified_diff(mutant: Mutant, source_bytes: bytes) -> str:
    """One mutant as a patch: `a/` and `b/` prefixes, project-relative, LF only."""
    before = source_bytes.decode("utf-8").splitlines(keepends=True)
    after = _apply(mutant, source_bytes).decode("utf-8").splitlines(keepends=True)
    return "".join(
        _terminated(line)
        for line in difflib.unified_diff(
            before,
            after,
            fromfile=f"a/{mutant.file}",
            tofile=f"b/{mutant.file}",
            n=CONTEXT_LINES,
        )
    )


def _terminated(line: str) -> str:
    """A diff line, with the note a patch needs when a file has no final newline.

    Keeping the line endings is what lets a missing final newline be seen at all,
    and the only lines that can arrive without one are the last of each side. A
    patch tool needs to be told, or the next line runs into this one.
    """
    if line.endswith("\n"):
        return line
    return f"{line}\n{NO_FINAL_NEWLINE}\n"


def _digest(content: bytes) -> str:
    return sha256(content).hexdigest()


def _string_map(value: object, path: Path, key: str) -> dict[str, str]:
    """A JSON object read as a map of text to text, or an error saying it is not.

    Raises:
        ValueError: The value is not an object whose every key and value is text.
    """
    if not isinstance(value, dict):
        raise ValueError(f"{path}: `{key}` is not an object of strings")
    mapping: dict[str, str] = {}
    for name, entry in cast("dict[object, object]", value).items():
        if not isinstance(name, str) or not isinstance(entry, str):
            raise ValueError(f"{path}: `{key}` is not an object of strings")
        mapping[name] = entry
    return mapping
