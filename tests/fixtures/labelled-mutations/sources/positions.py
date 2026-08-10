"""Translate manifest byte spans into the coordinates parso works in.

A manifest addresses code by raw byte offsets, while parso — and therefore
Cosmic Ray — addresses it by `(line, column)` pairs whose columns count
*characters*. This module owns that translation plus the two checks that stand
on it: whether a replacement is parseable Python, and whether a span lines up
with a real node (so the operator can match it at all).

Every function here assumes neutral validation already accepted the input:
UTF-8, LF line endings, and spans inside the file. A span that violates those
assumptions surfaces as an error rather than a plausible-looking wrong answer.

That division of labour has one edge worth naming: a carriage return inside a
replacement is **not** something the checks here can catch. Some spellings slip
through them intact — `"x = 2\\r\\n"` and a lone `"\\r"` are both accepted — and
what happens next depends on the span. Replacing an expression, the carriage
return disappears; replacing a whole statement, it reaches the file as a CRLF
line ending, which is exactly what a CRLF *file* is refused for. Neither is the
mutation that was described, and no check over the replacement alone can tell
which it will be, so rejecting carriage returns belongs to the neutral,
language-agnostic validation that already refuses CRLF files.

parso ships `py.typed`, but its tree API is annotated loosely enough that
pyright sees `Unknown` (`parse` returns `Unknown`, `BaseNode.children` is
`Unknown`, and the abstract `get_code`/`get_first_leaf` appear to return
`None`). The protocols below are the typed boundary: every unannotated parso
call is wrapped once, cast to a protocol, and never touched raw again — so the
rest of the pack, `cr_operator` included, works against real types.
"""

from bisect import bisect_right
from typing import Protocol, cast

from parso.grammar import load_grammar

from tremula_python.contracts import Span

Position = tuple[int, int]
"""A parso coordinate: 1-indexed line, 0-indexed column counted in characters."""

_FILLER_TYPES = frozenset({"endmarker", "newline"})


class ParsoLeaf(Protocol):
    """The parso leaf surface this pack uses: everything a node has, plus prefix.

    `prefix` is the whitespace and comments that precede the leaf. It is
    writable because grafting it onto a replacement node is what keeps
    `return a + b` from becoming `returnb - a`.
    """

    prefix: str

    @property
    def start_pos(self) -> Position: ...


class ParsoNode(Protocol):
    """The parso node surface this pack uses, whether the node is a leaf or not."""

    @property
    def type(self) -> str: ...

    @property
    def start_pos(self) -> Position: ...

    @property
    def end_pos(self) -> Position: ...

    def get_code(self, include_prefix: bool = True) -> str: ...

    def get_first_leaf(self) -> ParsoLeaf: ...


class ParsoIssue(Protocol):
    """One problem parso's error normalizer found in a parse tree."""

    @property
    def message(self) -> str: ...

    @property
    def start_pos(self) -> Position: ...


class ParsoGrammar(Protocol):
    """The parso grammar surface this pack uses, in the types it really has."""

    def parse(self, code: str) -> ParsoNode: ...

    def iter_errors(self, node: ParsoNode) -> list[ParsoIssue]: ...


def python_grammar() -> ParsoGrammar:
    """A typed view of parso's grammar for the running Python version.

    Cosmic Ray parses through `parso.parse`, which uses this same grammar, so
    the pack and the backend always agree on what the tree looks like.
    """
    # parso's own annotations stop at `Grammar[Unknown]`; this cast is the one
    # place that knows it, and the protocol carries the types onward.
    return cast(ParsoGrammar, load_grammar())  # pyright: ignore[reportUnknownArgumentType]


def parse_source(source: str) -> ParsoNode:
    """Parse `source` into a `file_input` node, the way Cosmic Ray does.

    parso never raises on invalid code: it recovers with error nodes. Use
    `validate_replacement` to find out whether the code was actually valid.
    """
    return python_grammar().parse(source)


def children_of(node: ParsoNode) -> list[ParsoNode]:
    """The node's children, or an empty list for a leaf."""
    children: object = getattr(node, "children", None)
    if not isinstance(children, list):
        return []
    return cast("list[ParsoNode]", children)


def significant_children(node: ParsoNode) -> list[ParsoNode]:
    """The node's children, minus the ones that carry no code of their own.

    Every `file_input` ends with a zero-width `endmarker`, and a `simple_stmt`
    ends with the `newline` that terminated it. Counting statements means
    counting everything else.
    """
    return [child for child in children_of(node) if child.type not in _FILLER_TYPES]


def byte_span_to_positions(source_bytes: bytes, span: Span) -> tuple[Position, Position]:
    """Convert a half-open byte span into parso start and end positions.

    Raises:
        ValueError: A span boundary is past the end of the file.
        UnicodeDecodeError: A span boundary falls inside a multi-byte character.
    """
    line_starts = _line_starts(source_bytes)
    return (
        _position_of(source_bytes, line_starts, span.start_byte),
        _position_of(source_bytes, line_starts, span.end_byte),
    )


def validate_replacement(replacement: str) -> list[str]:
    """Report why `replacement` cannot be injected, or nothing if it can.

    Three checks. First, syntax: parso recovers from invalid code instead of
    raising, so `iter_errors` is the only way to hear about it. Second, arity:
    `mutate` can return exactly one node, so a replacement that parses into two
    top-level statements is rejected here rather than silently losing one. Third,
    round trip: the node parsed back out must carry the whole replacement, which
    catches everything parso files under a leaf's `prefix` — trailing comments,
    leading comments, leading blank lines — since the operator overwrites that
    prefix with the original node's and the text there would vanish.

    An empty replacement is fine: it deletes the target node.
    """
    normalized = normalize_replacement(replacement)
    grammar = python_grammar()
    tree = grammar.parse(normalized)
    errors = [
        f"{issue.message} at line {issue.start_pos[0]}, column {issue.start_pos[1]}"
        for issue in grammar.iter_errors(tree)
    ]
    if errors:
        return errors

    statements = significant_children(tree)
    if len(statements) > 1:
        return [
            f"replacement must be a single top-level statement, found {len(statements)}: "
            + ", ".join(statement.type for statement in statements)
        ]
    if statements:
        return reparse_loss(statements[0], normalized)
    return []


def reparse_loss(extracted: ParsoNode, source: str) -> list[str]:
    """Report the text `extracted` would drop, or nothing if it carries all of it.

    parso parks comments and blank lines in the `prefix` of the leaf that follows
    them, and a prefix is not part of the node's own code. Injecting the node
    therefore leaves that text behind, which is why the comparison is against the
    whole replacement rather than the node alone.
    """
    injected = extracted.get_code(include_prefix=False)
    if injected == source:
        return []
    return [
        f"replacement would lose text: {source!r} is injected as {injected!r} "
        "(comments and blank lines around it are not part of the node)"
    ]


def node_for_span(source: str, start: Position, end: Position) -> ParsoNode | None:
    """The node whose extent is exactly `start`..`end`, or None if there is none.

    This is the language half of "can the operator match this span at all". The
    walk is pre-order, so the outermost node wins when a span belongs to more
    than one — which is also the order Cosmic Ray's visitor uses. Nodes sharing
    a span share their code, so any of them replaces to the same text.
    """
    for node in _walk(parse_source(source)):
        if node.start_pos == start and node.end_pos == end:
            return node
    return None


def normalize_replacement(replacement: str) -> str:
    """Strip trailing newlines so the parse shape matches a span's meaning.

    `parso.parse("x = 2")` has no `simple_stmt` wrapper while
    `parso.parse("x = 2\\n")` does, and a manifest span normally stops before
    the newline. Normalizing here makes both spellings of a replacement behave
    identically.
    """
    return replacement.rstrip("\n")


def _walk(node: ParsoNode) -> list[ParsoNode]:
    """The node and all its descendants, outermost first."""
    nodes = [node]
    for child in children_of(node):
        nodes.extend(_walk(child))
    return nodes


def _line_starts(source_bytes: bytes) -> list[int]:
    """Byte offset of the first byte of every line."""
    starts = [0]
    offset = source_bytes.find(b"\n")
    while offset != -1:
        starts.append(offset + 1)
        offset = source_bytes.find(b"\n", offset + 1)
    return starts


def _position_of(source_bytes: bytes, line_starts: list[int], offset: int) -> Position:
    # Slicing past the end would clamp, and clamping turns an out-of-range span
    # into a position that looks perfectly ordinary. An offset equal to the
    # length is legal: that is the end of the file.
    if offset > len(source_bytes):
        raise ValueError(
            f"byte offset {offset} is past the end of a {len(source_bytes)}-byte file"
        )
    line_index = bisect_right(line_starts, offset) - 1
    prefix = source_bytes[line_starts[line_index] : offset]
    return (line_index + 1, len(prefix.decode("utf-8")))
