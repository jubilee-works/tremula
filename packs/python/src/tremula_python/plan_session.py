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

Alongside the configuration this module computes what the run will be checked
against: the hash each target file must have while a given mutant is applied, and
the hash the others must keep. Those predictions are made with the operator's own
shaping helper, because a replacement and the text finally injected can differ by
a newline and a hand-rolled splice would condemn valid mutants.
"""

import json
import shlex
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
from typing import cast

from cosmic_ray.config import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    serialize_config,  # pyright: ignore[reportUnknownVariableType]
)

from tremula_python.contracts import Baseline, Manifest, Mutant, Stage
from tremula_python.cr_operator import FULL_OPERATOR_NAME, shaped_like_the_span
from tremula_python.errors import PackFailure
from tremula_python.positions import byte_span_to_positions, node_for_span
from tremula_python.run_layout import RunLayout, write_atomically

BACKSTOP_MARGIN_SECONDS = 10.0
"""How much longer Cosmic Ray waits than the runner it is running.

Enough for the runner to kill a hung process group, collect what it printed, and
write its own summary — its drain window plus room for the cost of starting up.
"""

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
    is how such a value quietly turns into a different one.
    """
    return serialize_config(
        {
            "module-path": list(target_files(manifest)),
            "timeout": effective_timeout + BACKSTOP_MARGIN_SECONDS,
            "test-command": test_command(layout, tests, effective_timeout),
            "excluded-modules": [],
            "distributor": {"name": "local"},
            "operators": {
                FULL_OPERATOR_NAME: [
                    _parameterization(mutant, project_root) for mutant in manifest.mutants
                ]
            },
        }
    )


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
    sources = {
        file: (project_root / file).read_bytes() for file in target_files(manifest)
    }
    return ExpectedHashes(
        base={file: _digest(source) for file, source in sources.items()},
        mutated={
            mutant.id: _digest(_apply(mutant, sources[mutant.file]))
            for mutant in manifest.mutants
        },
    )


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
        base=_hash_map(fields.get("base"), path, "base"),
        mutated=_hash_map(fields.get("mutated"), path, "mutated"),
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

    Predicting this is not quite a splice, and both departures come from how the
    operator really works. The replacement is shaped by the operator's own helper
    first, because the same replacement spelled with and without a trailing
    newline has to produce the same file. And a deletion reaches further back than
    its span: parso keeps the whitespace and comments before a statement in the
    statement's own prefix, so removing the node removes them too.

    Getting either wrong would not merely mispredict — it would report a mutant
    that was applied perfectly as a backend failure.
    """
    injected = shaped_like_the_span(mutant.replacement, mutant.original)
    start = mutant.span.start_byte
    if not injected:
        start -= _prefix_length(mutant, source_bytes)
    return source_bytes[:start] + injected.encode("utf-8") + source_bytes[mutant.span.end_byte :]


def _prefix_length(mutant: Mutant, source_bytes: bytes) -> int:
    """How many bytes of prefix the target node carries, as the parser sees it.

    Raises:
        PackFailure: The span matches no node, which the language checks are there
            to catch long before this point.
    """
    start, end = byte_span_to_positions(source_bytes, mutant.span)
    node = node_for_span(source_bytes.decode("utf-8"), start, end)
    if node is None:
        raise PackFailure(
            Stage.PLAN,
            "span_matches_no_node",
            f"mutant {mutant.id}: bytes {mutant.span.start_byte}..{mutant.span.end_byte} of "
            f"{mutant.file} are not exactly one syntax node, so what deleting them would "
            "leave behind cannot be predicted",
        )
    return len(node.get_first_leaf().prefix.encode("utf-8"))


def _digest(content: bytes) -> str:
    return sha256(content).hexdigest()


def _hash_map(value: object, path: Path, key: str) -> dict[str, str]:
    if not isinstance(value, dict):
        raise ValueError(f"{path}: `{key}` is not an object of hashes")
    mapping: dict[str, str] = {}
    for name, digest in cast("dict[object, object]", value).items():
        if not isinstance(name, str) or not isinstance(digest, str):
            raise ValueError(f"{path}: `{key}` is not an object of hashes")
        mapping[name] = digest
    return mapping
