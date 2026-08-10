"""Everything that has to be true before a single mutant is applied.

Two kinds of check live here. The environment checks ask whether this pack can
drive the Cosmic Ray it was built against: is the operator actually registered,
and is the installed version inside the pinned range. Neither is paranoia —
Cosmic Ray logs a plugin load failure and carries on, and the pack leans on
behaviors of 8.4.x that are not part of any published API.

The language checks ask whether the manifest describes mutations this language
can express: does the file still compile with each replacement in it, can each
replacement stand in for a single node, does each span line up with a node the
backend can match, and does the mutation change the program at all. Doing this
before the session exists is what gives a later missing work item its meaning —
with the spans pre-checked, a mutant that produces no job is an adapter bug
rather than a span that never had a chance.

*A replacement is judged by the file it makes, not by itself.* Reading it alone
answers a different question than the one that matters: `return errors` is no
module and parses as nothing on its own, and is exactly right inside a function.
Of 144 measured proposals, 34 were refused for failing to parse alone and all 34
compiled once put where they belong — while two that parsed alone broke the file
they went into, one of them badly enough to end a run as a runtime error. So the
whole file is compiled with the mutation spliced in, using the same splice the
operator's own module owns, and the file that is checked is therefore the file
that runs.

Everything the neutral, language-agnostic validation already guarantees — UTF-8,
LF endings, spans inside the file, `original` matching the bytes — is assumed
here.
"""

import ast
from collections.abc import Iterator, Sequence
from importlib.metadata import version
from pathlib import Path

from cosmic_ray.plugins import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    operator_names,  # pyright: ignore[reportUnknownVariableType]
)

from tremula_python.contracts import Manifest, Mutant, Stage
from tremula_python.cr_operator import FULL_OPERATOR_NAME, file_with_the_mutation
from tremula_python.errors import PackFailure
from tremula_python.positions import (
    byte_span_to_positions,
    injection_problems,
    node_for_span,
)
from tremula_python.refusals import Refusal, Refused

REPLACEMENT_DOES_NOT_COMPILE = "replacement_does_not_compile"
"""Code for a replacement the target file will not compile with."""

INVALID_REPLACEMENT = "invalid_replacement"
"""Code for a replacement that cannot stand in for a single node."""

SPAN_MATCHES_NO_NODE = "span_matches_no_node"
"""Code for a span that is not exactly one syntax node."""

MUTATION_IS_AST_EQUAL = "mutation_is_ast_equal"
"""Code for a mutation the parser cannot tell from the original."""

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
    """Check every mutant against the rules of the language, stopping at the first bad one.

    This is what the `validate` subcommand answers with, where the question is
    whether the manifest is usable and the first reason it is not is the answer.

    Raises:
        PackFailure: A replacement cannot be injected, a span does not line up
            with a node, or the mutation changes nothing the parser can see. The
            message names the mutant.
    """
    for mutant, source_bytes, tree in _each_mutant(manifest, project_root):
        _check_mutant(mutant, source_bytes, tree)


def refusals_in(manifest: Manifest, project_root: Path) -> Refused:
    """The same checks, asked of each mutant on its own, reported rather than raised.

    This is what a `run` asks, where the question is different: one mutation this
    language cannot express is a reason to leave that mutation out, not a reason
    to abandon the verdicts of every other mutant in the batch.
    """
    refused: Refused = {}
    for mutant, source_bytes, tree in _each_mutant(manifest, project_root):
        try:
            _check_mutant(mutant, source_bytes, tree)
        except PackFailure as failure:
            refused[mutant.id] = Refusal(failure.code, failure.message)
    return refused


def _each_mutant(
    manifest: Manifest, project_root: Path
) -> Iterator[tuple[Mutant, bytes, str | None]]:
    """Every mutant with its target file's bytes and the tree that file already has.

    Each file is read once and parsed once, because the last check compares every
    mutation against the tree of the file it is a mutation of.
    """
    sources: dict[str, bytes] = {}
    trees: dict[str, str | None] = {}
    for mutant in manifest.mutants:
        if mutant.file not in sources:
            sources[mutant.file] = (project_root / mutant.file).read_bytes()
            trees[mutant.file] = _tree_of(sources[mutant.file])
        yield mutant, sources[mutant.file], trees[mutant.file]


def _check_mutant(mutant: Mutant, source_bytes: bytes, tree: str | None) -> None:
    """Hold one mutant to the rules of the language, in the published order.

    The order is the one `capabilities` advertises, and the first failure is the
    one reported: the file, then the replacement's shape, then the span, then
    whether any of it made a difference.

    Raises:
        PackFailure: The mutant fails one of the four checks.
    """
    mutated = file_with_the_mutation(
        source_bytes,
        mutant.span.start_byte,
        mutant.span.end_byte,
        mutant.original,
        mutant.replacement,
    )
    _check_it_compiles(mutant, mutated)
    problems = injection_problems(mutant.replacement)
    if problems:
        raise PackFailure(
            Stage.VALIDATE,
            INVALID_REPLACEMENT,
            f"mutant {mutant.id}: {problems[0]}",
        )
    start, end = byte_span_to_positions(source_bytes, mutant.span)
    if node_for_span(source_bytes.decode("utf-8"), start, end) is None:
        raise PackFailure(
            Stage.VALIDATE,
            SPAN_MATCHES_NO_NODE,
            f"mutant {mutant.id}: bytes {mutant.span.start_byte}..{mutant.span.end_byte} of "
            f"{mutant.file} are not exactly one syntax node (line {start[0]}, column "
            f"{start[1]} to line {end[0]}, column {end[1]}); move the span to a node's "
            "boundaries",
        )
    _check_it_changes_the_tree(mutant, tree, mutated)


def _check_it_compiles(mutant: Mutant, mutated: bytes) -> None:
    """Compile the whole target file with this mutation spliced into it.

    `compile` rather than a parse, because a parse is not the whole of being
    valid Python: `ast.parse` accepts a function with two parameters of the same
    name and `compile` refuses it, and that exact mutation reached a measured run
    and ended it as a runtime error.

    The message carries the compiler's own words, which is what a caller asking
    the model again has to put in front of it.

    Raises:
        PackFailure: The file the mutation makes is not Python.
    """
    try:
        compile(mutated.decode("utf-8"), mutant.file, "exec")
    except (SyntaxError, ValueError) as error:
        raise PackFailure(
            Stage.VALIDATE,
            REPLACEMENT_DOES_NOT_COMPILE,
            f"mutant {mutant.id}: {mutant.file} does not compile with this replacement in "
            f"it — {error}; the replacement has to be valid where the span puts it, not "
            "only on its own",
        ) from error


def _check_it_changes_the_tree(mutant: Mutant, tree: str | None, mutated: bytes) -> None:
    """Refuse a mutation the parser cannot tell from the original.

    A replacement that only regroups an expression — `(a < b) and c` for
    `a < b and c` — makes a different file and the same program. Nothing a test
    suite does could distinguish the two, so reporting the mutant as survived
    would blame the suite for a difference that is not there. Comments and blank
    lines are already refused by the round-trip check; regrouping is what is left.

    The comparison is over `ast.dump`, not the parse tree the rest of this pack
    works in: parso keeps every token, so its trees differ whenever the text does,
    which is precisely the comparison this check exists not to make.

    Raises:
        PackFailure: The mutated file has the original's syntax tree.
    """
    if tree is None or tree != _tree_of(mutated):
        return
    raise PackFailure(
        Stage.VALIDATE,
        MUTATION_IS_AST_EQUAL,
        f"mutant {mutant.id}: this replacement leaves {mutant.file} with the syntax tree it "
        "already had, so no test could tell the two apart; change what the code does, not "
        "how it is written",
    )


def _tree_of(source_bytes: bytes) -> str | None:
    """A file's syntax tree as text, or None when it is not Python at all.

    Only the unmutated file can answer None here — a mutated one has compiled by
    the time it is asked — and that answer means the file was not Python before
    the mutation. Two trees cannot be the same when one of them does not exist,
    which is why the caller reads None as "different" rather than comparing it.
    """
    try:
        return ast.dump(ast.parse(source_bytes.decode("utf-8")))
    except (SyntaxError, ValueError):
        return None


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
