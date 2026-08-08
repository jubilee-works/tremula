"""What more than one test module needs: the fixture project and its manifest.

The identifier derivation lives here because two implementations of it have to
agree. The core computes it in Rust and refuses a manifest whose identifiers are
not the canonical derivation of the mutant's own fields, so a Python copy that
drifted would make every manifest built here unusable — silently, and only once
the core was in the loop. `test_pack_run_integration` therefore checks this
implementation against a vector the Rust suite pins independently.
"""

from collections.abc import Callable
from hashlib import sha256
from pathlib import Path
from shutil import copytree
from typing import Any

import pytest

collect_ignore_glob = ["fixtures/*"]
"""The fixture project holds a suite of its own; a run under test collects it, this one must not."""

PACK_PROJECT = Path(__file__).parent / "fixtures" / "pack_project"
"""A miniature project: two functions, three tests, one of them a boundary."""

MutantIdentifier = Callable[[str, int, int, str, str], str]
"""How a mutant identifier is derived: file, span, base file hash, replacement."""


def _canonical_mutant_id(
    file: str,
    start_byte: int,
    end_byte: int,
    base_file_sha256: str,
    replacement: str,
) -> str:
    """The identifier the manifest contract derives for a mutant.

    A SHA-256 over the fields that decide what the mutation is, joined by NUL
    bytes, with the byte offsets written as decimal text. Including the file's
    hash is what makes an identifier stale as soon as the file changes.
    """
    parts = (
        "tremula/mutant/v1",
        file,
        str(start_byte),
        str(end_byte),
        base_file_sha256,
        replacement,
    )
    return sha256(b"\0".join(part.encode("utf-8") for part in parts)).hexdigest()


def _mutant(file: str, source: str, original: str, replacement: str) -> dict[str, Any]:
    """One mutant over `source`, located by searching for the text it replaces.

    The search returns a character offset, which is the byte offset too because
    the fixture sources are ASCII.
    """
    start = source.index(original)
    end = start + len(original.encode("utf-8"))
    digest = sha256(source.encode("utf-8")).hexdigest()
    return {
        "id": _canonical_mutant_id(file, start, end, digest, replacement),
        "file": file,
        "base_file_sha256": digest,
        "span": {"start_byte": start, "end_byte": end},
        "original": original,
        "replacement": replacement,
    }


@pytest.fixture(scope="session")
def canonical_mutant_id() -> MutantIdentifier:
    """The canonical identifier derivation, for tests that build their own mutants."""
    return _canonical_mutant_id


@pytest.fixture(scope="session")
def copy_pack_project() -> Callable[[Path], Path]:
    """Copy the fixture project somewhere writable.

    Always a copy: a run mutates its target files in place, and a crash can leave
    one mutated. Nothing under version control should be exposed to that.
    """

    def copy(destination: Path) -> Path:
        copytree(PACK_PROJECT, destination)
        return destination

    return copy


@pytest.fixture(scope="session")
def pack_project_manifest() -> dict[str, Any]:
    """The golden manifest for the fixture project: one mutant caught, one not.

    The first moves the boundary the suite pins, so the suite fails. The second
    moves the boundary nothing pins, so the suite passes and the mutant survives.
    """
    source = (PACK_PROJECT / "schedule.py").read_text(encoding="utf-8")
    return {
        "schema_version": "0.1",
        "language": "python",
        "base": {"revision": None},
        "mutants": [
            _mutant("schedule.py", source, "other_start < end", "other_start <= end"),
            _mutant("schedule.py", source, "minutes >= 60", "minutes > 60"),
        ],
    }
