"""A Cosmic Ray operator that injects one mutant described by a manifest.

Cosmic Ray's built-in operators decide for themselves what to change. This one
does not: it is handed a location, the text it expects to find there, and the
text to put in its place, and it changes that one thing or nothing at all.
Cosmic Ray carries those arguments from the session TOML through the work-db and
re-instantiates the operator at execution time, so the whole mutant travels on
supported plumbing.

Two details of that plumbing shape the code:

* Cosmic Ray matches by walking every node of every module in the session, and
  it never tells the operator which file it is looking at. Position equality is
  therefore the only thing that distinguishes two identical snippets, and
  `original` is a second, independent check on top of it.
* Occurrence counting lives in Cosmic Ray's visitor, which applies at most one
  mutation per walk. The operator needs no "already applied" guard.

Cosmic Ray ships no `py.typed`, so its `Operator` base class arrives untyped and
its abstract methods — docstring-only bodies with no return annotation — look to
a type checker like they return `None`. The overrides below annotate themselves
fully and work against the parso protocols from `positions`, which confines the
untyped region to the base class and the suppressions to the four lines that
touch it.
"""

from collections.abc import Iterator, Sequence

from cosmic_ray.operators.operator import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    Argument,
    Example,
    Operator,
)

from tremula_python.positions import (
    ParsoNode,
    Position,
    normalize_replacement,
    parse_source,
    reparse_loss,
    significant_children,
)

PROVIDER_NAME = "tremula"
"""The provider's name, which is the entry point's name in the pack's metadata.

Changing it here changes nothing on its own: the two spellings have to agree, and
`preflight` fails the run when Cosmic Ray cannot resolve the composed name.
"""

OPERATOR_NAME = "spec-mutation"
"""The operator's name within this provider."""

FULL_OPERATOR_NAME = f"{PROVIDER_NAME}/{OPERATOR_NAME}"
"""How Cosmic Ray addresses the operator: provider name, a slash, operator name.

This is the spelling that goes in a session's TOML keys and comes back out of the
work-db, so it is also what a job is matched against when results are collected.
"""


class TremulaOperator(Operator):
    """Replace the code at one exact position with one exact replacement."""

    def __init__(
        self,
        mutant_id: str,
        start_line: int,
        start_col: int,
        end_line: int,
        end_col: int,
        original: str,
        replacement: str,
    ) -> None:
        """Take one manifest mutant, already converted to parso coordinates."""
        self.mutant_id = mutant_id
        self.original = original
        self.replacement = replacement
        self._start: Position = (start_line, start_col)
        self._end: Position = (end_line, end_col)

    @classmethod
    def arguments(cls) -> Sequence[Argument]:
        """Declare the TOML keys Cosmic Ray parameterizes this operator with.

        The names are the constructor's keywords, and every value is a string or
        an integer so it survives TOML and then the work-db's JSON.
        """
        return (
            Argument("mutant_id", "Manifest id of the mutant, for correlation."),
            Argument("start_line", "1-indexed line the target starts on."),
            Argument("start_col", "0-indexed character column the target starts at."),
            Argument("end_line", "1-indexed line the target ends on."),
            Argument("end_col", "0-indexed character column the target ends at."),
            Argument("original", "Source text the target node must currently have."),
            Argument("replacement", "Source text to put in its place; empty deletes it."),
        )

    @classmethod
    def examples(cls) -> Sequence[Example]:  # pyright: ignore[reportIncompatibleMethodOverride] - the abstract base has no return annotation, so it appears to return None
        """No examples: every mutation this operator makes comes from a manifest."""
        return ()

    def mutation_positions(  # pyright: ignore[reportIncompatibleMethodOverride] - the abstract base has no return annotation, so it appears to return None
        self, node: ParsoNode
    ) -> Iterator[tuple[Position, Position]]:
        """Yield the target extent if `node` is the node this mutant addresses.

        Both checks matter. Position alone would match the wrong file, since
        Cosmic Ray walks every module in the session with no file context. Text
        alone would match every copy of the snippet. The comparison uses
        `include_prefix=False` because the default `get_code()` carries the
        preceding whitespace and comments, which a manifest span excludes.
        """
        if (node.start_pos, node.end_pos) != (self._start, self._end):
            return
        if node.get_code(include_prefix=False) != self.original:
            return
        if _is_redundant_root(node):
            return
        yield (self._start, self._end)

    def mutate(self, node: ParsoNode, index: int) -> ParsoNode | None:  # pyright: ignore[reportIncompatibleMethodOverride] - the abstract base has no return annotation, so it appears to return None
        """Build the node that replaces `node`, or None to delete it.

        Args:
            node: The matched node, whose leading whitespace is inherited.
            index: Which of the yielded positions to mutate. This operator
                yields at most one, so there is nothing to choose.

        Raises:
            ValueError: The replacement is not a single statement, or injecting
                it would drop part of its text. The pack's `validate_replacement`
                rejects both before Cosmic Ray runs, so reaching either means
                validation was skipped — and failing loudly beats writing a file
                that quietly lost something.
        """
        del index
        source = shaped_like_the_span(self.replacement, self.original)
        statements = significant_children(parse_source(source))
        if not statements:
            return None
        if len(statements) > 1:
            raise ValueError(
                f"mutant {self.mutant_id}: replacement is {len(statements)} statements, "
                "which cannot replace a single node"
            )
        losses = reparse_loss(statements[0], source)
        if losses:
            raise ValueError(f"mutant {self.mutant_id}: {losses[0]}")
        return _adopt(statements[0], node)


class TremulaProvider:
    """Cosmic Ray operator provider exposing this pack's single operator."""

    def __iter__(self) -> Iterator[str]:
        """Name every operator this provider offers."""
        return iter((OPERATOR_NAME,))

    def __getitem__(self, name: str) -> type[TremulaOperator]:
        """Look an operator class up by name."""
        if name != OPERATOR_NAME:
            raise KeyError(name)
        return TremulaOperator


def _is_redundant_root(node: ParsoNode) -> bool:
    """Whether `node` is a tree root that one of its own children already covers.

    A file holding a single statement gives `file_input` and that statement the
    same extent and the same code, so a span over the whole file matches twice —
    with or without a trailing newline, since the shared child is `simple_stmt`
    in the first case and the bare statement in the second. Two matches would
    become two Cosmic Ray jobs for one manifest mutant, breaking the one-job-per-
    mutant invariant that makes a missing job mean a real adapter bug.

    The child is the match that is kept, because replacing the root is worse than
    redundant: deleting it removes the tree Cosmic Ray then tries to read the
    mutated code from.
    """
    if node.type != "file_input":
        return False
    extent = (node.start_pos, node.end_pos)
    return any(
        (child.start_pos, child.end_pos) == extent for child in significant_children(node)
    )


def shaped_like_the_span(replacement: str, original: str) -> str:
    """The exact text to parse so the replacement's shape matches the span's.

    parso decides whether to wrap a statement in `simple_stmt` by whether the
    code ends in a newline, so the replacement has to end the way the span does.
    A span normally stops before the newline, and then a trailing newline in the
    replacement would inject a second one. A span that *swallowed* its newline
    needs the opposite: without one, the statement on the next line is spliced
    onto this one — `x = 2y = 2`. Either way both spellings of a replacement
    produce the same file.

    Public because that last sentence is a promise something else has to keep:
    the session planner predicts each mutated file's hash, and predicting it by
    splicing the raw replacement into the span would disagree with what the
    operator writes whenever the two differ by a newline — which would report a
    perfectly good mutant as a backend error.
    """
    normalized = normalize_replacement(replacement)
    if normalized and original.endswith("\n"):
        return normalized + "\n"
    return normalized


def _adopt(replacement_node: ParsoNode, original_node: ParsoNode) -> ParsoNode:
    """Prepare a freshly parsed node to stand in for `original_node`.

    Two adjustments. A `simple_stmt` wrapper is unwrapped when the node being
    replaced is not itself one, because keeping it would duplicate the
    surrounding newline; the unwrap is skipped when the wrapper holds more than
    one statement, so nothing is silently dropped. Then the original's leading
    whitespace is grafted on, without which `return a + b` becomes
    `returnb - a`.
    """
    node = replacement_node
    if node.type == "simple_stmt" and original_node.type != "simple_stmt":
        inner = significant_children(node)
        if len(inner) == 1:
            node = inner[0]
    node.get_first_leaf().prefix = original_node.get_first_leaf().prefix
    return node
