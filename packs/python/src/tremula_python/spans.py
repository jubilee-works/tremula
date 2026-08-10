"""Where a mutation may land in one source file.

The answer this module gives is positional only: where every function begins and
ends, where its body statements are, and which stretches of that body carry no
behaviour a mutation could change. Nothing here decides what is worth mutating.

Two things about the machinery are worth knowing.

`ast` reports a column as a byte offset into the line's UTF-8 encoding, so an
absolute offset is the line's first byte plus the column. That is the opposite of
parso, whose columns count characters, which is why `positions` — built for
parso, and for the manifest spans the backend consumes — has no part in this
module and its small offset table lives here instead.

The neutral checks the core performs before it calls the pack have not run here.
`spans` is called directly, by a generator that has no manifest yet and therefore
nothing for the core to have validated, so this module reads the file itself and
refuses whatever it cannot describe honestly: bytes that are not UTF-8, an
encoding declaration or a carriage return the core would reject the file for
anyway, and a path that leads out of the project or through a link.
"""

import ast
from collections.abc import Iterator
from hashlib import sha256
from pathlib import Path

from tremula_python import target_file
from tremula_python.contracts import (
    CONTRACT_VERSION,
    ExcludedKind,
    ExcludedSpan,
    FunctionSpan,
    Span,
    SpansReport,
    Stage,
)
from tremula_python.errors import PackFailure

_Function = ast.FunctionDef | ast.AsyncFunctionDef
"""The two ways a file spells a function. A lambda is an expression, not one."""


def report(project_root: Path, relative: str) -> SpansReport:
    """Describe the functions of `relative`, a file of the project at `project_root`.

    Raises:
        PackFailure: The path does not name a readable file of this project, or
            the file is not Python this pack can parse.
    """
    source = target_file.read_bytes(project_root, relative, Stage.SPANS)
    text = target_file.decoded(source, relative, Stage.SPANS)
    offsets = _Offsets(source)
    return SpansReport(
        schema_version=CONTRACT_VERSION,
        file=relative,
        file_sha256=sha256(source).hexdigest(),
        functions=_functions(target_file.parsed(text, relative, Stage.SPANS), offsets),
    )


def as_document(found: SpansReport) -> str:
    """The report as the one line of stdout its caller reads.

    Compact JSON with no line breaks in it: the protocol reserves the last line of
    stdout for one document, so this is a wire format and not a form laid out for
    a person. The published examples under `contracts/examples/spans/` are these
    same documents indented for reading, which makes them something other than
    these bytes — what the pack's tests hold is the round trip, that an example
    read back and written out here is the exact line the subcommand printed.

    An empty list is left out rather than written as `[]`, which is how the
    contract's own examples spell "there are none" and what the core's producer
    of the same document emits.
    """
    return found.model_dump_json(exclude_defaults=True)


def _functions(module: ast.Module, offsets: "_Offsets") -> list[FunctionSpan]:
    """Every function in the module, in the order the file spells them.

    Sorting by where a function starts is what puts a function before the ones
    nested inside it: nothing can begin before the definition that contains it.
    """
    found: list[FunctionSpan] = []
    _collect(module, (), found, offsets)
    found.sort(key=lambda function: function.span.start_byte)
    return found


def _collect(
    node: ast.AST,
    scope: tuple[str, ...],
    found: list[FunctionSpan],
    offsets: "_Offsets",
) -> None:
    """Add every function under `node`, naming each for the scope it sits in.

    The descent goes through every node rather than through statement lists only,
    because a function can be defined anywhere a statement can stand — under an
    `if`, inside a `with`, in the body of a class in the body of a function.
    """
    for child in ast.iter_child_nodes(node):
        if isinstance(child, _Function):
            found.append(_function_span(child, ".".join((*scope, child.name)), offsets))
            _collect(child, (*scope, child.name, "<locals>"), found, offsets)
        elif isinstance(child, ast.ClassDef):
            _collect(child, (*scope, child.name), found, offsets)
        else:
            _collect(child, scope, found, offsets)


def _function_span(function: _Function, qualified_name: str, offsets: "_Offsets") -> FunctionSpan:
    """One function's geography.

    `ast` puts a function's own position at its `def` or `async def`, so its
    decorators — which have positions of their own — are outside `span` already.
    The body is where its statements are: from the start of the first to the end
    of the last, which leaves the signature, and the comments and blank lines
    below it, outside `body_span`.
    """
    return FunctionSpan(
        qualified_name=qualified_name,
        span=offsets.span_of(function),
        body_span=Span(
            start_byte=offsets.start_of(function.body[0]),
            end_byte=offsets.end_of(function.body[-1]),
        ),
        excluded=_excluded(function, offsets),
    )


def _excluded(function: _Function, offsets: "_Offsets") -> list[ExcludedSpan]:
    """What inside the function's body carries no behaviour to change.

    Only what this function owns. A nested function's docstring and annotations
    are reported against that function's own entry, whose whole span a consumer
    subtracts from this body in any case.
    """
    found: list[ExcludedSpan] = []
    _add_docstring(function, found, offsets)
    for node in _owned(function):
        if isinstance(node, ast.AnnAssign):
            found.append(
                ExcludedSpan(kind=ExcludedKind.ANNOTATION, span=offsets.span_of(node.annotation))
            )
        elif isinstance(node, ast.ClassDef):
            # A class defined inside a function is part of what the function owns;
            # its methods are not, because each of them has an entry of its own.
            _add_docstring(node, found, offsets)
    found.sort(key=lambda excluded: excluded.span.start_byte)
    return found


def _add_docstring(
    node: _Function | ast.ClassDef,
    found: list[ExcludedSpan],
    offsets: "_Offsets",
) -> None:
    """Exclude the node's documentation string, if it opens with one."""
    statement = _docstring_statement(node)
    if statement is not None:
        found.append(ExcludedSpan(kind=ExcludedKind.DOCSTRING, span=offsets.span_of(statement)))


def _docstring_statement(node: _Function | ast.ClassDef) -> ast.Expr | None:
    """The statement holding the node's docstring, or None when it has none.

    The whole statement is the answer, not the string inside it: a mutation that
    replaced the string would leave an expression standing where a docstring was,
    and the bytes to leave alone are the ones the statement occupies.
    """
    first = node.body[0]
    if (
        isinstance(first, ast.Expr)
        and isinstance(first.value, ast.Constant)
        and isinstance(first.value.value, str)
    ):
        return first
    return None


def _owned(function: _Function) -> Iterator[ast.AST]:
    """Every node of the function's body except what a nested function owns."""
    for statement in function.body:
        yield from _owned_from(statement)


def _owned_from(node: ast.AST) -> Iterator[ast.AST]:
    """The node and its descendants, stopping at a function that owns its own."""
    yield node
    if isinstance(node, _Function):
        return
    for child in ast.iter_child_nodes(node):
        yield from _owned_from(child)


class _Offsets:
    """Absolute byte offsets for the line-and-column positions `ast` reports."""

    def __init__(self, source: bytes) -> None:
        """Record where every line of `source` starts."""
        starts = [0]
        at = source.find(b"\n")
        while at != -1:
            starts.append(at + 1)
            at = source.find(b"\n", at + 1)
        self._line_starts = starts

    def span_of(self, node: ast.stmt | ast.expr) -> Span:
        """The half-open byte range the node occupies."""
        return Span(start_byte=self.start_of(node), end_byte=self.end_of(node))

    def start_of(self, node: ast.stmt | ast.expr) -> int:
        """Where the node's first byte is."""
        return self._at(node.lineno, node.col_offset, node)

    def end_of(self, node: ast.stmt | ast.expr) -> int:
        """Where the node ends, one byte past its last.

        Raises:
            PackFailure: The parser gave the node no end position.
        """
        if node.end_lineno is None or node.end_col_offset is None:
            raise _unpositioned(node)
        return self._at(node.end_lineno, node.end_col_offset, node)

    def _at(self, lineno: int, column: int, node: ast.stmt | ast.expr) -> int:
        """The absolute offset of a 1-indexed line and a column counted in bytes.

        Raises:
            PackFailure: The line is not one this file has.
        """
        if not 1 <= lineno <= len(self._line_starts):
            raise _unpositioned(node)
        return self._line_starts[lineno - 1] + column


def _unpositioned(node: ast.AST) -> PackFailure:
    """A node the parser placed nowhere this file has.

    Nothing parsed from a file should reach this, which is why it says so plainly
    rather than guessing an offset: a guess would be a span pointing at code that
    is not the code it names.
    """
    return PackFailure(
        Stage.SPANS,
        "unpositioned_node",
        f"the parser reported a `{type(node).__name__}` without a position in the file, "
        "so the spans of this file cannot be trusted",
    )
