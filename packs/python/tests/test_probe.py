"""What a probe observes, and everything it refuses to claim.

Every case below is one a measured run met for real: an algebraic rewrite whose
witness was fabricated, a boundary the suite missed, a difference that is only an
exception, a difference that is only printed output, and the ways a witness turns
out not to be runnable at all. The fixture spells each concern as its own small
function so that a failure names the concern.

The comparison has a second group of cases of its own. A probe may only ever
report a difference two processes really disagree about, so a value it cannot say
that of — anything but the types whose equality is their content — is undecided,
and a rendering that tells apart values that are `==` would be a difference this
tool invented.
"""

import json
from pathlib import Path

import pytest

from tremula_python import probe
from tremula_python.__main__ import main
from tremula_python.contracts import Ending, ProbeOutcome, ProbeReport, Undecided
from tremula_python.errors import PackFailure
from tremula_python.witness import MAX_NODES, function_named

PROJECT = Path(__file__).resolve().parent / "fixtures" / "probe_project"
CONTRACTS = Path(__file__).resolve().parents[3] / "contracts"
FILE = "behaviour.py"


def _witness(original: str, replacement: str, call: str, file: str = FILE) -> probe.Witness:
    """A witness aimed at the one place `original` occurs in the fixture."""
    source = (PROJECT / file).read_bytes()
    target = original.encode("utf-8")
    start = source.index(target)
    assert source.count(target) == 1, f"`{original}` is not in `{file}` exactly once"
    return probe.Witness(
        file=file,
        start_byte=start,
        end_byte=start + len(target),
        replacement=replacement,
        call=call,
    )


def _probe(original: str, replacement: str, call: str) -> ProbeReport:
    return probe.probe(PROJECT, _witness(original, replacement, call))


def test_a_boundary_the_suite_missed_is_observed_as_a_difference() -> None:
    # Two ranges that only touch: the half-open reading says they do not overlap,
    # and an inclusive `<=` says they do.
    report = _probe("start < other_end", "start <= other_end", "overlaps(30, 60, 0, 30)")
    assert report.outcome is ProbeOutcome.DIFFERS
    assert report.original is not None
    assert report.mutant is not None
    assert (report.original.value, report.mutant.value) == ("False", "True")
    assert report.original.ended is Ending.RETURNED
    assert report.runs_per_side == probe.RUNS_PER_SIDE


def test_an_algebraic_rewrite_is_not_told_apart_by_the_input_it_named() -> None:
    # The measured case: the model claimed ceiling division and this rewrite differ
    # for negative minutes, and they are the same function for every positive slot.
    report = _probe("-(-minutes // slot)", "(minutes + slot - 1) // slot", "slots(90, 30)")
    assert report.outcome is ProbeOutcome.INDISTINGUISHABLE
    assert report.original is not None
    assert report.mutant is not None
    assert report.original.value == report.mutant.value == "3"


def test_a_negative_literal_is_an_argument_a_probe_will_run() -> None:
    report = _probe("-(-minutes // slot)", "(minutes + slot - 1) // slot", "slots(-40, 7)")
    assert report.outcome is ProbeOutcome.INDISTINGUISHABLE


def test_a_difference_that_is_only_an_exception_is_a_difference() -> None:
    report = _probe('marker["timed_out"]', 'marker.get("timed_out")', "outcome_of({})")
    assert report.outcome is ProbeOutcome.DIFFERS
    assert report.original is not None
    assert report.mutant is not None
    assert report.original.ended is Ending.RAISED
    assert report.original.type_name == "KeyError"
    assert report.original.message == "'timed_out'"
    assert report.mutant.ended is Ending.RETURNED
    assert report.mutant.type_name == "Outcome"


def test_a_member_of_an_enumeration_is_compared_by_its_name() -> None:
    # Both sides return `Outcome.TIMED_OUT`, but out of two different loadings of
    # the module in two different processes. Compared by identity there is no
    # answer; compared by name there is, and it is the right one.
    report = _probe(
        'marker["timed_out"]', 'marker.get("timed_out")', 'outcome_of({"timed_out": True})'
    )
    assert report.outcome is ProbeOutcome.INDISTINGUISHABLE


def test_a_difference_that_is_only_printed_is_a_difference() -> None:
    report = _probe('f"hello {name}"', 'f"HELLO {name}"', 'announce("ada")')
    assert report.outcome is ProbeOutcome.DIFFERS
    assert report.original is not None
    assert report.mutant is not None
    assert report.original.value == report.mutant.value == "3", "the return is the same"
    assert report.original.stdout == "hello ada\n"
    assert report.mutant.stdout == "HELLO ada\n"


def test_a_module_that_imports_a_neighbour_is_still_probed() -> None:
    report = _probe("doubled(number)", "doubled(number) + 1", "twice(4)")
    assert report.outcome is ProbeOutcome.DIFFERS
    assert report.original is not None
    assert report.original.value == "8"


def test_a_witness_that_never_returns_is_undecided_rather_than_a_hung_pack() -> None:
    report = probe.probe(
        PROJECT,
        _witness("number -= 1", "number -= 0", "countdown(3)"),
        timeout=2.0,
    )
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.TIMED_OUT


def test_a_side_that_disagrees_with_itself_is_undecided() -> None:
    report = _probe("random.random() * scale", "random.random() * (scale + 1)", "pick(2)")
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.NONDETERMINISTIC
    assert report.original is None, "there is no one account of a side that varies"


def test_a_value_that_compares_by_identity_is_undecided() -> None:
    report = _probe("Token(name)", 'Token(name + "!")', 'mint("ada")')
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.INCOMPARABLE


def test_a_value_whose_equality_is_its_own_is_not_compared_by_its_rendering() -> None:
    # The two values here are `==` and render differently. A comparison by rendering
    # would report a difference the language says is not one, and a triage would
    # publish it as a mutation distinguished at the level of the function.
    report = _probe("Loose(name)", 'Loose(name + "!")', 'loose("ada")')
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.INCOMPARABLE


def test_an_int_of_a_type_of_its_own_is_not_compared_as_an_int() -> None:
    # The whitelist is of exact types, not of what a value is an instance of: this is
    # an `int` by inheritance and its equality is not the one `int` has.
    report = _probe("Counted(number)", "Counted(number + 1)", "counted(3)")
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.INCOMPARABLE


def test_a_dataclass_is_not_compared_although_it_defines_equality() -> None:
    # A dataclass does have equality of its own, and its rendering is even faithful to
    # it. It is still a type the two processes each define for themselves, and the rule
    # is the type of the value rather than a guess about whoever wrote it.
    report = _probe("Slot(start, end)", "Slot(start, end + 1)", "slot(0, 30)")
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.INCOMPARABLE


def test_an_enumeration_that_redefined_equality_is_not_compared_by_name() -> None:
    # A member's name stands for the member only while the enumeration has not made
    # two names one value. This one has, so the name is no longer a comparison.
    report = _probe("Lenient(which)", 'Lenient("b")', 'lenient("a")')
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.INCOMPARABLE


def test_a_value_that_contains_itself_is_undecided_rather_than_a_dead_probe() -> None:
    # A rendering built out of a value's parts does not terminate on a value that is
    # one of its own parts. That is a fact about the value: it has to arrive as
    # `undecided`, and never as a language pack that died on the way.
    report = _probe("[size]", "[size + 1]", "looping(2)")
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.INCOMPARABLE


def test_two_spellings_of_one_number_are_not_a_difference() -> None:
    # `Decimal("1.50") == Decimal("1.500")`, and the two print differently. Inside a
    # dict, inside a value the comparison is built out of part by part.
    report = _probe("Decimal(amount)", 'Decimal(amount + "0")', 'priced("1.50")')
    assert report.outcome is ProbeOutcome.INDISTINGUISHABLE


def test_two_spellings_of_one_instant_are_not_a_difference() -> None:
    # Two aware datetimes are `==` when they are the same instant, whatever zone each
    # is written in — and their renderings are not equal at all.
    report = _probe(
        "datetime(2026, 1, 1, tzinfo=timezone.utc)",
        "datetime(2025, 12, 31, 23, tzinfo=timezone(timedelta(hours=-1)))",
        "moment(5)",
    )
    assert report.outcome is ProbeOutcome.INDISTINGUISHABLE


def test_a_value_built_only_out_of_comparable_parts_is_still_compared() -> None:
    report = _probe("Decimal(amount)", 'Decimal(amount + "1")', 'priced("1.50")')
    assert report.outcome is ProbeOutcome.DIFFERS
    assert report.original is not None
    assert report.original.type_name == "dict"


@pytest.mark.parametrize(
    "call",
    [
        "overlaps(0, 30, 30, other_end)",
        "overlaps(0, 30, 30, len([1, 2]))",
        "overlaps(0, 30, 30, 30 + 30)",
        "Calendar().busy(30)",
        "overlaps(*[0, 30, 30, 60])",
        "overlaps(0, 30, 30, 60); overlaps(1, 2, 3, 4)",
        "not a call at all(",
    ],
)
def test_a_witness_a_probe_will_not_evaluate_is_undecided(call: str) -> None:
    report = _probe("start < other_end", "start <= other_end", call)
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.UNSAFE_WITNESS
    assert report.original is None and report.mutant is None


def test_what_a_witness_may_say_is_a_list_of_syntax_and_these_are_its_edges() -> None:
    # Each of these was probed against the interpreter this pack runs under before it
    # was pinned, because what `literal_eval` happens to accept is not the rule: it
    # takes `set()`, which is a call, and it takes a hundred thousand elements.
    ran = {
        "holds({1, 2})": 2,
        'holds(b"abc")': 3,
        "holds({})": 0,
        "holds([-1, 2.5, None, True, (1,), {}])": 6,
    }
    for call, held in ran.items():
        report = _probe("len(items)", "len(items) + 1", call)
        assert report.outcome is ProbeOutcome.DIFFERS, call
        assert report.original is not None
        assert report.original.value == str(held), call
    refused = ["holds(set())", "holds(frozenset({1}))", "holds([1 + 1])", "holds(1j)"]
    for call in refused:
        report = _probe("len(items)", "len(items) + 1", call)
        assert report.outcome is ProbeOutcome.UNDECIDED, call
        assert report.undecided is Undecided.UNSAFE_WITNESS, call


def test_a_witness_of_more_syntax_than_the_cap_is_refused_before_anything_runs() -> None:
    inside = ", ".join("0" for _ in range(MAX_NODES // 2))
    assert function_named(f"holds([{inside}])") == "holds"
    enormous = ", ".join("0" for _ in range(100_000))
    assert function_named(f"holds([{enormous}])") is None
    report = _probe("len(items)", "len(items) + 1", f"holds([{enormous}])")
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.UNSAFE_WITNESS


def test_a_method_has_no_receiver_a_witness_could_name() -> None:
    report = _probe("minutes > 0", "minutes >= 0", "busy(30)")
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.METHOD


def test_a_name_the_module_does_not_define_is_undecided() -> None:
    report = _probe("start < other_end", "start <= other_end", "overlapping(0, 30, 30, 60)")
    assert report.outcome is ProbeOutcome.UNDECIDED
    assert report.undecided is Undecided.NO_SUCH_FUNCTION


def test_a_span_the_file_does_not_have_is_a_failure_rather_than_a_verdict() -> None:
    witness = probe.Witness(
        file=FILE, start_byte=0, end_byte=10_000_000, replacement="x", call="overlaps(0, 1, 0, 1)"
    )
    with pytest.raises(PackFailure) as raised:
        probe.probe(PROJECT, witness)
    assert raised.value.code == "span_outside_file"


def test_the_original_side_is_read_from_the_root_it_was_given(tmp_path: Path) -> None:
    # Triage points the probe at a run's snapshot, so the bytes probed have to be
    # the ones under that root and not the ones anywhere else. A root whose copy of
    # the function says something different has to produce a different answer.
    (tmp_path / FILE).write_text(
        "def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:\n"
        "    return False\n",
        encoding="utf-8",
    )
    witness = _witness("start < other_end", "start <= other_end", "overlaps(0, 30, 30, 60)")
    source = (tmp_path / FILE).read_bytes()
    start = source.index(b"False")
    elsewhere = probe.Witness(
        file=FILE,
        start_byte=start,
        end_byte=start + len(b"False"),
        replacement="True",
        call=witness.call,
    )
    report = probe.probe(tmp_path, elsewhere)
    assert report.outcome is ProbeOutcome.DIFFERS
    assert report.original is not None
    assert report.original.value == "False"


def test_no_difference_is_not_evidence_of_equivalence_and_the_protocol_says_so() -> None:
    protocol = (CONTRACTS / "pack-protocol.md").read_text(encoding="utf-8")
    assert "`indistinguishable` is not a finding of equivalence" in protocol
    assert "difference at the level of the function" in protocol


def test_the_subcommand_prints_one_report_as_its_last_line(
    capsys: pytest.CaptureFixture[str],
) -> None:
    witness = _witness("start < other_end", "start <= other_end", "overlaps(30, 60, 0, 30)")
    code = main(
        [
            "probe",
            "--file",
            witness.file,
            "--project",
            str(PROJECT),
            "--span",
            f"{witness.start_byte}:{witness.end_byte}",
            "--replacement",
            witness.replacement,
            "--call",
            witness.call,
        ]
    )
    assert code == 0
    last = capsys.readouterr().out.strip().splitlines()[-1]
    report = ProbeReport.model_validate_json(last)
    assert report.outcome is ProbeOutcome.DIFFERS
    assert report.file == FILE
    assert json.loads(last)["call"] == witness.call


@pytest.mark.parametrize("span", ["", "5", "5:5", "9:5", "a:b", "-1:4"])
def test_a_span_that_is_not_a_range_of_bytes_is_refused(
    span: str, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(
        [
            "probe",
            "--file",
            FILE,
            "--project",
            str(PROJECT),
            "--span",
            span,
            "--replacement",
            "x",
            "--call",
            "overlaps(0, 1, 0, 1)",
        ]
    )
    assert code == 2
    reported = json.loads(capsys.readouterr().out.strip().splitlines()[-1])
    assert reported["error"]["code"] == "invalid_arguments"
    assert reported["error"]["stage"] == "preflight"
