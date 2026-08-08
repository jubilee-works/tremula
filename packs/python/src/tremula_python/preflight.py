"""Everything that has to be true before a single mutant is applied.

Two kinds of check live here. The environment checks ask whether this pack can
drive the Cosmic Ray it was built against: is the operator actually registered,
and is the installed version inside the pinned range. Neither is paranoia —
Cosmic Ray logs a plugin load failure and carries on, and the pack leans on
behaviors of 8.4.x that are not part of any published API.

The language checks ask whether the manifest describes mutations this language
can express: does each replacement parse into something injectable, and does
each span line up with a node the backend can match. Doing this before the
session exists is what gives a later missing work item its meaning — with the
spans pre-checked, a mutant that produces no job is an adapter bug rather than a
span that never had a chance.

Everything the neutral, language-agnostic validation already guarantees — UTF-8,
LF endings, spans inside the file, `original` matching the bytes — is assumed
here.
"""

from collections.abc import Sequence
from importlib.metadata import version
from pathlib import Path

from cosmic_ray.plugins import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    operator_names,  # pyright: ignore[reportUnknownVariableType]
)

from tremula_python.contracts import Manifest, Mutant, Stage
from tremula_python.cr_operator import FULL_OPERATOR_NAME
from tremula_python.errors import PackFailure
from tremula_python.positions import (
    byte_span_to_positions,
    node_for_span,
    validate_replacement,
)

DISTRIBUTION = "cosmic_ray"
"""The backend distribution whose version is pinned."""

MINIMUM_VERSION = (8, 4, 6)
"""Oldest Cosmic Ray this pack is known to work with."""

BELOW_VERSION = (8, 5)
"""First Cosmic Ray release this pack is not known to work with."""


def check_environment() -> None:
    """Refuse to run against a backend this pack cannot drive.

    Raises:
        PackFailure: The operator is not registered, or the installed Cosmic Ray
            is outside the pinned range.
    """
    check_operator_is_registered(registered_operator_names())
    check_cosmic_ray_version(version(DISTRIBUTION))


def registered_operator_names() -> tuple[str, ...]:
    """Every operator Cosmic Ray can currently resolve, by full name."""
    return operator_names()


def check_operator_is_registered(names: Sequence[str]) -> None:
    """Confirm this pack's operator is among `names`.

    Raises:
        PackFailure: It is not — which usually means the pack is installed in a
            different environment from the Cosmic Ray being asked to run it.
    """
    if FULL_OPERATOR_NAME in names:
        return
    raise PackFailure(
        Stage.PREFLIGHT,
        "operator_not_registered",
        f"Cosmic Ray does not know the operator `{FULL_OPERATOR_NAME}`; install "
        "tremula-python into the same environment as cosmic-ray and check that "
        "its entry point loads",
    )


def check_cosmic_ray_version(spelling: str) -> None:
    """Confirm `spelling` is a Cosmic Ray release this pack supports.

    Raises:
        PackFailure: The version is outside `MINIMUM_VERSION`..`BELOW_VERSION`.
    """
    if MINIMUM_VERSION <= _release(spelling) < BELOW_VERSION:
        return
    raise PackFailure(
        Stage.PREFLIGHT,
        "unsupported_cosmic_ray",
        f"cosmic-ray {spelling} is installed, but this pack supports "
        f"{_spelled(MINIMUM_VERSION)} up to (not including) {_spelled(BELOW_VERSION)}; "
        "pin cosmic-ray to that range",
    )


def validate_language(manifest: Manifest, project_root: Path) -> None:
    """Check every mutant against the rules of the language.

    Raises:
        PackFailure: A replacement cannot be injected, or a span does not line
            up with a node. The message names the mutant.
    """
    sources: dict[str, bytes] = {}
    for mutant in manifest.mutants:
        if mutant.file not in sources:
            sources[mutant.file] = (project_root / mutant.file).read_bytes()
        _check_mutant(mutant, sources[mutant.file])


def _check_mutant(mutant: Mutant, source_bytes: bytes) -> None:
    problems = validate_replacement(mutant.replacement)
    if problems:
        raise PackFailure(
            Stage.VALIDATE,
            "invalid_replacement",
            f"mutant {mutant.id}: {problems[0]}",
        )
    start, end = byte_span_to_positions(source_bytes, mutant.span)
    if node_for_span(source_bytes.decode("utf-8"), start, end) is None:
        raise PackFailure(
            Stage.VALIDATE,
            "span_matches_no_node",
            f"mutant {mutant.id}: bytes {mutant.span.start_byte}..{mutant.span.end_byte} of "
            f"{mutant.file} are not exactly one syntax node (line {start[0]}, column "
            f"{start[1]} to line {end[0]}, column {end[1]}); move the span to a node's "
            "boundaries",
        )


def _release(spelling: str) -> tuple[int, ...]:
    """The leading numeric release segment of a version, as integers.

    Compared as a tuple, never as text: `8.4.10` is newer than `8.4.6` and
    `8.10.0` is newer than `8.5`, and string ordering says the opposite of both.
    Anything the pack cannot read as a release — a suffix, an empty string —
    stops the tuple there, so an unreadable version falls outside the range
    instead of looking like the oldest supported one.
    """
    segments: list[int] = []
    for part in spelling.split("."):
        digits = ""
        for character in part:
            if not character.isdigit():
                break
            digits += character
        if not digits:
            break
        segments.append(int(digits))
    return tuple(segments)


def _spelled(release: tuple[int, ...]) -> str:
    return ".".join(str(segment) for segment in release)
