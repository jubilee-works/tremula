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
from itertools import takewhile
from pathlib import Path

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

BYTE_ORDER_MARK = b"\xef\xbb\xbf"
"""What a UTF-8 encoder may prepend to a file, and what the core refuses."""

COOKIE_LINES = 2
"""How many leading lines may carry an encoding declaration, as the core counts them."""

_Function = ast.FunctionDef | ast.AsyncFunctionDef
"""The two ways a file spells a function. A lambda is an expression, not one."""


def report(project_root: Path, relative: str) -> SpansReport:
    """Describe the functions of `relative`, a file of the project at `project_root`.

    Raises:
        PackFailure: The path does not name a readable file of this project, or
            the file is not Python this pack can parse.
    """
    root = _canonical_root(project_root)
    source = _read(_project_file(root, relative), relative)
    text = _decoded(source, relative)
    offsets = _Offsets(source)
    return SpansReport(
        schema_version=CONTRACT_VERSION,
        file=relative,
        file_sha256=sha256(source).hexdigest(),
        functions=_functions(_parsed(text, relative), offsets),
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


def _canonical_root(project_root: Path) -> Path:
    """The project root as the filesystem sees it.

    Everything below is measured against this, so it is resolved once and here:
    a root reached through a link of its own is still the root the caller named.

    Raises:
        PackFailure: The root does not resolve to a directory that exists.
    """
    try:
        resolved = project_root.resolve(strict=True)
    except OSError as error:
        raise PackFailure(
            Stage.SPANS,
            "project_root_unresolvable",
            f"the project root `{project_root}` cannot be resolved: {error}",
        ) from error
    if not resolved.is_dir():
        raise PackFailure(
            Stage.SPANS,
            "project_root_unresolvable",
            f"the project root `{project_root}` is not a directory",
        )
    return resolved


def _project_file(root: Path, relative: str) -> Path:
    """The file `relative` names, refused unless it is really the project's own.

    The spelling is checked as text first, because a host's own path syntax is
    not the contract's: on this one `..\\escape.py` is a single ordinary name.
    Then every step of the way down is checked for being a link, because a
    relative path with no `..` in it still leaves the project when the filesystem
    cooperates — and a report over a file the project does not own describes
    somebody else's code as this project's mutation targets.

    Raises:
        PackFailure: The path is not a plain project-relative POSIX path, leads
            through a link, or lands outside the project.
    """
    names = relative.split("/")
    if not _is_plain_posix_spelling(relative) or any(
        name in {"", ".", ".."} for name in names
    ):
        raise PackFailure(
            Stage.SPANS,
            "invalid_target_path",
            f"`{relative}` is not a project-relative POSIX path; spell the file the "
            "way a manifest would, as `src/module.py`",
        )
    path = root
    for name in names:
        path = path / name
        if path.is_symlink():
            raise PackFailure(
                Stage.SPANS,
                "target_reached_through_link",
                f"`{relative}` reaches the file through the link `{name}`; describe the "
                "file where it really lives",
            )
    # The walk above has ruled out every spelling that leaves the project, so this
    # is the check that does not depend on that enumeration being complete.
    if not path.resolve().is_relative_to(root):
        raise PackFailure(
            Stage.SPANS,
            "target_outside_project",
            f"`{relative}` lands outside the project at `{root}`",
        )
    return path


def _is_plain_posix_spelling(relative: str) -> bool:
    """Whether the text is a non-empty relative POSIX path with no empty segment."""
    return bool(relative) and not (
        "\\" in relative
        or relative.startswith("/")
        or relative.endswith("/")
        or "//" in relative
    )


def _read(path: Path, relative: str) -> bytes:
    """Read the file, keeping "it is not there" apart from "I cannot read it".

    Raises:
        PackFailure: The file is missing, or it is there and unreadable.
    """
    try:
        return path.read_bytes()
    except FileNotFoundError as error:
        raise PackFailure(
            Stage.SPANS,
            "target_missing",
            f"`{relative}` is not a file of this project",
        ) from error
    except OSError as error:
        raise PackFailure(
            Stage.SPANS,
            "target_unreadable",
            f"`{relative}` cannot be read: {error}",
        ) from error


def _decoded(source: bytes, relative: str) -> str:
    """The file as text, refused unless it is the plain UTF-8 the contract assumes.

    Each refusal here is one the core's own validation would reach for the same
    file, and they are taken in the core's order, so the failure a caller is told
    about is the one it would be told about with a manifest in hand. A report over
    a file the core refuses would describe offsets no run could ever use them for.

    A byte-order mark is looked for before the decode, because it decodes
    perfectly well into a character that then sits in front of the first
    statement. An encoding declaration is looked for after it, because a file can
    announce latin-1 in bytes that are also valid UTF-8.

    Raises:
        PackFailure: The bytes carry a byte-order mark, are not UTF-8, declare an
            encoding that is not UTF-8, or hold a carriage return.
    """
    if source.startswith(BYTE_ORDER_MARK):
        raise PackFailure(
            Stage.SPANS,
            "target_has_byte_order_mark",
            f"`{relative}` starts with a byte-order mark; save it as plain UTF-8",
        )
    try:
        text = source.decode("utf-8")
    except UnicodeDecodeError as error:
        raise PackFailure(
            Stage.SPANS,
            "target_not_utf8",
            f"`{relative}` is not UTF-8: {error}",
        ) from error
    if _declares_other_encoding(text):
        raise PackFailure(
            Stage.SPANS,
            "target_declares_other_encoding",
            f"`{relative}` declares an encoding other than UTF-8; only plain UTF-8 "
            "sources are supported",
        )
    if "\r" in text:
        raise PackFailure(
            Stage.SPANS,
            "target_has_carriage_return",
            f"`{relative}` uses carriage returns; tremula works on files with "
            "line-feed endings only",
        )
    return text


def _declares_other_encoding(text: str) -> bool:
    """Whether a leading comment declares an encoding that is not UTF-8.

    The reading is the core's, step for step, because being stricter here would
    refuse a file a run would go on to accept and being laxer would report spans
    over one it would not: the first two lines only, comments only, `coding`
    followed by a colon or an equals sign, and the name folded so that `UTF-8`,
    `utf_8`, and `utf8` are one name. Lines are split on the line feed alone, so
    the other characters Python calls line breaks do not open a third line the
    core would not have looked at.
    """
    for line in text.split("\n")[:COOKIE_LINES]:
        if not line.lstrip().startswith("#"):
            continue
        declared = _declared_encoding(line)
        if declared is not None and declared != "utf8":
            return True
    return False


def _declared_encoding(line: str) -> str | None:
    """The encoding a comment declares, folded for comparison, or None for none.

    Only the first `coding` in the line is considered, and only when a colon or an
    equals sign follows it directly — which is what makes the word on its own, in
    prose or in a path, no declaration at all.
    """
    keyword = line.find("coding")
    if keyword == -1:
        return None
    value = line[keyword + len("coding") :]
    if not value.startswith((":", "=")):
        return None
    name = "".join(takewhile(_names_an_encoding, value[1:].lstrip()))
    if not name:
        return None
    return name.lower().replace("-", "").replace("_", "")


def _names_an_encoding(character: str) -> bool:
    """Whether the character can stand in an encoding's name."""
    return (character.isascii() and character.isalnum()) or character in {"-", "_", "."}


def _parsed(text: str, relative: str) -> ast.Module:
    """Parse the file, or say that it is not Python.

    A null byte is refused by the parser as a `ValueError` rather than a syntax
    error, and it is the same answer for the caller either way: nothing here can
    describe this file.

    Raises:
        PackFailure: The text is not a Python module.
    """
    try:
        return ast.parse(text, filename=relative)
    except (SyntaxError, ValueError) as error:
        raise PackFailure(
            Stage.SPANS,
            "target_does_not_parse",
            f"`{relative}` is not valid Python: {error}",
        ) from error


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
