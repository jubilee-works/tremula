"""The record of which mutants a run refused before it could try them.

A run that has to leave a mutant out writes the reason here, and `collect` reads
it back so that the results document can say *why* a mutation was never applied
rather than only that it was not. Nothing else in the run directory has anywhere
to keep that: the session holds jobs, and a refused mutant has none to hold.
"""

from pathlib import Path

import pytest

from tremula_python import refusals
from tremula_python.run_layout import RunLayout


def _layout(tmp_path: Path) -> RunLayout:
    layout = RunLayout.at(tmp_path / "20260810T090000Z-3b1f8c")
    layout.prepare()
    return layout


def test_a_written_record_reads_back_as_it_was_written(tmp_path: Path) -> None:
    layout = _layout(tmp_path)
    written = {
        "first": refusals.Refusal("span_matches_no_node", "mutant first: nowhere to land"),
        "second": refusals.Refusal("unserializable_mutant", "mutant second: text lost in TOML"),
    }

    refusals.write(layout, written)

    assert refusals.read(layout) == written


def test_a_run_that_refused_nothing_reads_back_as_nothing_refused(tmp_path: Path) -> None:
    layout = _layout(tmp_path)
    refusals.write(layout, {})

    assert refusals.read(layout) == {}


def test_a_run_directory_without_the_record_reads_back_the_same_way(tmp_path: Path) -> None:
    # A run that never got as far as writing one, and every run directory made
    # before this file existed. Neither refused anything a reader can name.
    assert refusals.read(_layout(tmp_path)) == {}


@pytest.mark.parametrize(
    "document",
    [
        '["first"]',
        '{"first": "just a message"}',
        '{"first": {"code": "x"}}',
    ],
)
def test_a_document_that_is_not_a_record_of_refusals_is_refused(
    tmp_path: Path, document: str
) -> None:
    # Read straight into `backend_raw`, so a document that is not this shape would
    # put whatever it is into a results file the core parses.
    layout = _layout(tmp_path)
    layout.refusals.write_text(document, encoding="utf-8")

    with pytest.raises(ValueError, match="refusals.json"):
        refusals.read(layout)
