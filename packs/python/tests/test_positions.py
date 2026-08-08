import pytest

from tremula_python.contracts import Span
from tremula_python.positions import (
    byte_span_to_positions,
    node_for_span,
    validate_replacement,
)


def _span(start: int, end: int) -> Span:
    return Span(start_byte=start, end_byte=end)


def test_an_ascii_span_maps_to_parser_positions() -> None:
    source = "x = 1 + 2\n"
    assert source.encode()[4:9] == b"1 + 2"
    assert byte_span_to_positions(source.encode(), _span(4, 9)) == ((1, 4), (1, 9))


def test_a_leading_korean_comment_does_not_shift_the_next_line() -> None:
    # The comment line is 15 bytes but 7 characters, so a line table built on
    # bytes and columns measured in characters must not be mixed up.
    source = "# 한글 주석\nreturn a\n"
    assert source.encode()[23:24] == b"a"
    assert byte_span_to_positions(source.encode(), _span(23, 24)) == ((2, 7), (2, 8))


def test_columns_count_characters_not_bytes() -> None:
    # `x` sits at byte 15 but character 11: parso columns are character counts,
    # so a byte-based column would point at the wrong place.
    source = 's = "한글" + x\n'
    assert source.encode()[15:16] == b"x"
    positions = byte_span_to_positions(source.encode(), _span(15, 16))
    assert positions == ((1, 11), (1, 12))
    assert positions != ((1, 15), (1, 16))


def test_a_span_across_two_lines_maps_both_ends() -> None:
    source = "z = (a\n     + b)\n"
    assert source.encode()[5:15] == b"a\n     + b"
    assert byte_span_to_positions(source.encode(), _span(5, 15)) == ((1, 5), (2, 8))


def test_a_span_at_the_first_byte_starts_at_the_origin() -> None:
    source = "x = 1\n"
    assert byte_span_to_positions(source.encode(), _span(0, 1)) == ((1, 0), (1, 1))


def test_a_span_boundary_inside_a_character_is_rejected() -> None:
    # Neutral validation should have caught this; the propagated decode error
    # keeps a bad span from silently producing a plausible position.
    source = 's = "한"\n'
    with pytest.raises(UnicodeDecodeError):
        byte_span_to_positions(source.encode(), _span(0, 6))


def test_a_span_ending_before_the_newline_stops_on_the_first_line() -> None:
    source = "x = 1\n"
    assert byte_span_to_positions(source.encode(), _span(4, 5)) == ((1, 4), (1, 5))


def test_a_span_ending_at_a_file_without_a_trailing_newline() -> None:
    source = "x = 1"
    assert byte_span_to_positions(source.encode(), _span(0, 5)) == ((1, 0), (1, 5))


def test_a_span_reaching_past_the_end_of_the_file_is_rejected() -> None:
    # Clamping would answer with a position that looks entirely reasonable, and
    # the mutant would then fail to match for no visible reason.
    source = "x = 1\n"
    with pytest.raises(ValueError, match="past the end"):
        byte_span_to_positions(source.encode(), _span(4, 99))


def test_a_valid_replacement_reports_no_errors() -> None:
    assert validate_replacement("x = 1\n") == []


def test_a_syntactically_broken_replacement_reports_an_error() -> None:
    # `parso.parse` recovers from anything, so `iter_errors` is the only check.
    assert validate_replacement("def f(:\n") != []


def test_a_replacement_with_two_top_level_statements_is_rejected() -> None:
    # This parses cleanly, but `mutate` can only return one node.
    assert validate_replacement("x = 2\ny = 3") != []


def test_an_empty_replacement_is_allowed_because_it_deletes() -> None:
    assert validate_replacement("") == []


@pytest.mark.parametrize(
    "replacement",
    [
        "x = 2  # keep me",  # the comment ends up in the endmarker's prefix
        "# gone\nx = 2",  # a leading comment is prefix, and the graft overwrites it
        "\n\nx = 2",  # so are leading blank lines
    ],
)
def test_a_replacement_that_would_lose_text_is_rejected(replacement: str) -> None:
    # parso keeps comments and blank lines in a leaf's `prefix`, and the operator
    # replaces the prefix with the original node's. Anything living there is
    # dropped silently, so it has to be refused before the run starts.
    assert validate_replacement(replacement) != []


@pytest.mark.parametrize(
    "replacement",
    [
        "(1\n     + 2)",  # a multiline expression is one node and keeps its layout
        "x = 2; y = 3",  # semicolons stay inside one `simple_stmt`
        "x = 2\n",  # the trailing newline is normalized away, not lost
        "",  # deletion
    ],
)
def test_a_replacement_that_survives_a_reparse_is_accepted(replacement: str) -> None:
    assert validate_replacement(replacement) == []


def test_a_node_is_found_for_a_span_that_covers_it_exactly() -> None:
    node = node_for_span("x = 1 + 2\n", (1, 4), (1, 9))
    assert node is not None
    assert node.type == "arith_expr"
    assert node.get_code(include_prefix=False) == "1 + 2"


def test_no_node_is_found_for_a_span_that_straddles_nodes() -> None:
    assert node_for_span("x = 1 + 2\n", (1, 6), (1, 9)) is None
