"""Golden tests for the operator, driven through Cosmic Ray's real mutation walk.

Each case asserts on the *whole* mutated file, because the bugs this operator
can cause are invisible in a node-level assertion: a dropped prefix turns
`return a + b` into `returnb - a`, and a kept `simple_stmt` wrapper injects a
blank line.
"""

from inspect import signature
from pathlib import Path
from typing import cast, get_type_hints

import pytest
from cosmic_ray.ast import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    ast_nodes,  # pyright: ignore[reportUnknownVariableType]
)
from cosmic_ray.commands.init import (  # pyright: ignore[reportMissingTypeStubs] - same
    init,  # pyright: ignore[reportUnknownVariableType]
)
from cosmic_ray.mutating import (  # pyright: ignore[reportMissingTypeStubs] - same
    MutationVisitor,
)
from cosmic_ray.operators.operator import (  # pyright: ignore[reportMissingTypeStubs] - same
    Operator,
)
from cosmic_ray.plugins import (  # pyright: ignore[reportMissingTypeStubs] - same
    operator_names,
)
from cosmic_ray.work_db import (  # pyright: ignore[reportMissingTypeStubs] - same
    WorkDB,
    use_db,  # pyright: ignore[reportUnknownVariableType]
)
from cosmic_ray.work_item import (  # pyright: ignore[reportMissingTypeStubs] - same
    WorkItem,
)

from tremula_python.contracts import Span
from tremula_python.cr_operator import TremulaOperator, TremulaProvider
from tremula_python.positions import (
    ParsoNode,
    Position,
    byte_span_to_positions,
    node_for_span,
    parse_source,
)

RETURN_SOURCE = "def f(a, b):\n    return a + b\n"
OPERATOR_FULL_NAME = "tremula/spec-mutation"


def _operator(
    start: Position,
    end: Position,
    original: str,
    replacement: str,
) -> TremulaOperator:
    return TremulaOperator(
        mutant_id="m1",
        start_line=start[0],
        start_col=start[1],
        end_line=end[0],
        end_col=end[1],
        original=original,
        replacement=replacement,
    )


def _mutate(source: str, operator: TremulaOperator) -> str | None:
    """Apply `operator` to `source` through Cosmic Ray and return the whole file.

    Cosmic Ray spells the third parameter `occurence`, so it is passed
    positionally: a keyword argument would raise `TypeError`. Occurrence 0 is
    what an exactly-matching operator always gets, and the visitor applies at
    most one mutation per walk.
    """
    result = MutationVisitor.mutate_code(source, cast(Operator, operator), 0)  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    return cast("str | None", result)


def _positions_over_the_tree(
    source: str, operator: TremulaOperator
) -> list[tuple[Position, Position]]:
    """Every position the operator yields across a whole tree walk.

    This is the number Cosmic Ray's init turns into occurrences, and one mutant
    has to mean one job — so this count is the 1:1 invariant made observable.
    """
    nodes = cast("list[ParsoNode]", list(ast_nodes(parse_source(source))))  # pyright: ignore[reportUnknownArgumentType]
    return [position for node in nodes for position in operator.mutation_positions(node)]


def test_an_expression_is_replaced_and_keeps_its_leading_space() -> None:
    operator = _operator((2, 11), (2, 16), "a + b", "b - a")
    assert _mutate(RETURN_SOURCE, operator) == "def f(a, b):\n    return b - a\n"


def test_a_span_after_a_korean_comment_mutates_where_it_was_converted() -> None:
    source = "# 한글 주석\ndef f(a, b):\n    return a + b\n"
    assert source.encode()[40:45] == b"a + b"
    start, end = byte_span_to_positions(source.encode(), Span(start_byte=40, end_byte=45))
    assert (start, end) == ((3, 11), (3, 16))
    operator = _operator(start, end, "a + b", "b - a")
    expected = "# 한글 주석\ndef f(a, b):\n    return b - a\n"
    assert _mutate(source, operator) == expected


def test_a_multiline_replacement_is_injected_verbatim() -> None:
    operator = _operator((1, 4), (1, 9), "1 + 2", "(1\n     + 2)")
    assert _mutate("x = 1 + 2\n", operator) == "x = (1\n     + 2)\n"


def test_replacing_a_statement_does_not_inject_a_blank_line() -> None:
    operator = _operator((1, 0), (1, 5), "x = 1", "x = 2")
    assert _mutate("x = 1\n", operator) == "x = 2\n"


def test_a_trailing_newline_in_the_replacement_changes_nothing() -> None:
    without = _operator((1, 0), (1, 5), "x = 1", "x = 2")
    with_newline = _operator((1, 0), (1, 5), "x = 1", "x = 2\n")
    assert _mutate("x = 1\n", without) == _mutate("x = 1\n", with_newline) == "x = 2\n"


def test_an_empty_replacement_deletes_the_node_and_leaves_its_newline() -> None:
    operator = _operator((1, 0), (1, 5), "x = 1", "")
    assert _mutate("x = 1\ny = 2\n", operator) == "\ny = 2\n"


def test_deleting_a_whole_statement_takes_its_newline_with_it() -> None:
    operator = _operator((1, 0), (2, 0), "x = 1\n", "")
    assert _mutate("x = 1\ny = 2\n", operator) == "y = 2\n"


def test_replacing_a_span_that_swallowed_its_newline_keeps_the_line_break() -> None:
    # The span covers the statement *and* the newline that ended it, so the
    # replacement has to end the line too. Without that, the statement after it
    # is spliced onto the same line: `x = 2y = 2`.
    source = "x = 1\ny = 2\n"
    without = _operator((1, 0), (2, 0), "x = 1\n", "x = 2")
    with_newline = _operator((1, 0), (2, 0), "x = 1\n", "x = 2\n")
    assert _mutate(source, without) == _mutate(source, with_newline) == "x = 2\ny = 2\n"


def test_deleting_an_expression_may_leave_code_that_will_not_compile() -> None:
    # Syntactic validity is not this layer's job: a mutant that breaks the
    # module is reported through the runner, not hidden here.
    operator = _operator((1, 4), (1, 9), "1 + 2", "")
    assert _mutate("x = 1 + 2\n", operator) == "x =\n"


def test_a_span_covering_the_whole_file_matches_once() -> None:
    # `file_input` and the statement inside it have the same extent when a file
    # holds one statement, so a naive match would yield two positions and Cosmic
    # Ray would create two jobs for one mutant.
    source = "x = 1\n"
    operator = _operator((1, 0), (2, 0), source, "x = 2")
    assert _positions_over_the_tree(source, operator) == [((1, 0), (2, 0))]
    assert _mutate(source, operator) == "x = 2\n"


def test_a_whole_file_span_without_a_trailing_newline_matches_once() -> None:
    # The duplication is not about newlines: with no newline the shared extent is
    # `file_input` and `expr_stmt` instead.
    source = "x = 1"
    operator = _operator((1, 0), (1, 5), source, "x = 2")
    assert _positions_over_the_tree(source, operator) == [((1, 0), (1, 5))]
    assert _mutate(source, operator) == "x = 2"


def test_deleting_the_whole_file_empties_it_instead_of_crashing() -> None:
    # Matching `file_input` here would delete the root of the tree, and Cosmic Ray
    # would fail reading the code back off of None.
    operator = _operator((1, 0), (2, 0), "x = 1\n", "")
    assert _mutate("x = 1\n", operator) == ""


def test_cosmic_ray_creates_exactly_one_job_for_a_whole_file_mutant(tmp_path: Path) -> None:
    # The same invariant through the real init walk, which is what actually
    # numbers occurrences and writes the jobs a run will execute.
    module = tmp_path / "module.py"
    module.write_text("x = 1\n", encoding="utf-8")
    arguments = {
        "mutant_id": "m-1",
        "start_line": 1,
        "start_col": 0,
        "end_line": 2,
        "end_col": 0,
        "original": "x = 1\n",
        "replacement": "x = 2",
    }
    with use_db(tmp_path / "work.db", WorkDB.Mode.create) as work_db:
        init([module], work_db, {"tremula/spec-mutation": [arguments]})  # pyright: ignore[reportUnknownArgumentType]
        stored: tuple[WorkItem, ...] = work_db.work_items

    ours = [item for item in stored if item.mutations[0].operator_name == OPERATOR_FULL_NAME]
    assert len(ours) == 1
    assert ours[0].mutations[0].occurrence == 0
    assert ours[0].mutations[0].operator_args == arguments


def test_a_mismatched_original_produces_no_mutation() -> None:
    operator = _operator((1, 0), (1, 5), "x = 9", "x = 2")
    assert _mutate("x = 1\n", operator) is None


def test_only_the_targeted_copy_of_repeated_text_is_mutated() -> None:
    source = "def f(a, b):\n    x = a + b\n    y = a + b\n    return x + y\n"
    operator = _operator((3, 8), (3, 13), "a + b", "b - a")
    expected = "def f(a, b):\n    x = a + b\n    y = b - a\n    return x + y\n"
    assert _mutate(source, operator) == expected


def test_a_semicolon_joined_replacement_keeps_every_statement() -> None:
    operator = _operator((1, 0), (1, 5), "x = 1", "x = 2; y = 3")
    assert _mutate("x = 1\n", operator) == "x = 2; y = 3\n"


def test_a_replacement_that_would_lose_text_is_refused_by_the_operator() -> None:
    # `validate_replacement` rejects these before a run starts. Reaching the
    # operator means validation was skipped, and failing loudly beats writing a
    # file that quietly lost the comment.
    operator = _operator((1, 0), (1, 5), "x = 1", "x = 2  # keep me")
    with pytest.raises(ValueError, match="keep me"):
        _mutate("x = 1\n", operator)


def test_get_code_includes_the_prefix_unless_it_is_asked_not_to() -> None:
    # The reason the operator compares against `include_prefix=False`: the
    # default form carries the leading whitespace and would never match.
    node = node_for_span(RETURN_SOURCE, (2, 11), (2, 16))
    assert node is not None
    assert node.get_code() == " a + b"
    assert node.get_code(include_prefix=False) == "a + b"


def test_the_declared_arguments_are_the_constructor_keywords() -> None:
    # Cosmic Ray builds the operator with `operator_class(**operator_args)`
    # straight from the TOML keys, so the two lists must not drift.
    names = [argument.name for argument in TremulaOperator.arguments()]
    assert names == [
        "mutant_id",
        "start_line",
        "start_col",
        "end_line",
        "end_col",
        "original",
        "replacement",
    ]
    assert names == list(signature(TremulaOperator).parameters)


def test_every_argument_survives_a_round_trip_through_toml_and_json() -> None:
    # Arguments travel as TOML values and then as JSON in the work-db, so only
    # the scalar types both formats agree on are allowed.
    hints = get_type_hints(TremulaOperator.__init__)
    del hints["return"]
    assert set(hints.values()) == {str, int}


def test_the_operator_has_no_examples() -> None:
    # `examples()` is abstract, so it must exist; the operator is data-driven
    # and has nothing to demonstrate on its own.
    assert TremulaOperator.examples() == ()


def test_the_provider_offers_exactly_the_spec_mutation_operator() -> None:
    provider = TremulaProvider()
    assert tuple(provider) == ("spec-mutation",)
    assert provider["spec-mutation"] is TremulaOperator


def test_the_operator_is_registered_with_cosmic_ray() -> None:
    assert OPERATOR_FULL_NAME in operator_names()
