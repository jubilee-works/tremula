"""What the pack says about where a mutation may land, and what it refuses to say.

The golden fixture project holds non-ASCII text before and inside its functions,
which is the regression this suite exists for: `ast` counts columns in bytes, and
a table that counted characters would agree with it on every ASCII file and drift
on every other one.

The subcommand is exercised as a real subprocess wherever the contract is about
what the core reads — the exit code, the last line of stdout, an empty stderr —
and in-process where the subject is what the report says.
"""

import json
import subprocess
import sys
from hashlib import sha256
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from tremula_python import spans
from tremula_python.contracts import SpansReport

CONTRACTS = Path(__file__).resolve().parents[3] / "contracts"

SPANS_PROJECT = Path(__file__).parent / "fixtures" / "spans_project"
"""A project whose files are the ones the published examples describe."""

# Every published example, and the file of the fixture project it describes.
EXAMPLES = [
    ("spans/simple.json", "src/scheduling/overlap.py"),
    ("spans/decorated.json", "src/billing/invoice.py"),
    ("spans/async_nested.json", "src/scheduling/sync.py"),
]


def _contract(relative: str) -> dict[str, Any]:
    return json.loads((CONTRACTS / relative).read_text(encoding="utf-8"))


def _pack(*arguments: str) -> subprocess.CompletedProcess[str]:
    """Invoke the pack the way the core does: as a module, in this interpreter."""
    return subprocess.run(
        [sys.executable, "-m", "tremula_python", *arguments],
        capture_output=True,
        text=True,
        check=False,
    )


def _spans(project: Path, relative: str) -> subprocess.CompletedProcess[str]:
    return _pack("spans", "--file", relative, "--project", str(project))


def _last_line(output: str) -> dict[str, Any]:
    document: dict[str, Any] = json.loads(output.splitlines()[-1])
    return document


def _printed(project: Path, relative: str) -> str:
    """The line the subcommand prints, as text, with the protocol's rules checked first."""
    completed = _spans(project, relative)
    assert (completed.returncode, completed.stderr) == (0, "")
    lines = completed.stdout.splitlines()
    assert len(lines) == 1
    return lines[0]


def _document(project: Path, relative: str) -> dict[str, Any]:
    """The report the subcommand prints, read back as the document it is."""
    document: dict[str, Any] = json.loads(_printed(project, relative))
    return document


def _refusal(project: Path, relative: str) -> dict[str, str]:
    """The failure the subcommand reports, as the core would read it."""
    completed = _spans(project, relative)
    assert (completed.returncode, completed.stderr) == (2, "")
    document = _last_line(completed.stdout)
    jsonschema.validate(instance=document, schema=_contract("schemas/pack-error.schema.json"))
    error: dict[str, str] = document["error"]
    assert error["stage"] == "spans"
    return error


def _bytes_of(relative: str, span: dict[str, int]) -> bytes:
    source = (SPANS_PROJECT / relative).read_bytes()
    return source[span["start_byte"] : span["end_byte"]]


@pytest.mark.parametrize(("example", "relative"), EXAMPLES)
def test_the_published_example_is_the_document_the_subcommand_reports(
    example: str, relative: str
) -> None:
    # The examples are the contract's own reference for this document, so they are
    # not a second opinion about the fixture project: they are this pack's answer.
    assert _document(SPANS_PROJECT, relative) == _contract(f"examples/{example}")


@pytest.mark.parametrize(("example", "relative"), EXAMPLES)
def test_the_published_example_serializes_to_the_line_the_subcommand_prints(
    example: str, relative: str
) -> None:
    # The example is indented on disk and the printed line is compact, so the two
    # are not the same bytes and nothing here claims they are. What is pinned is
    # the step the comparison above parses away: read the example back and this
    # pack writes it as exactly the line it printed, so a change in how the
    # document is serialized cannot pass for a change in nothing.
    written = SpansReport.model_validate(_contract(f"examples/{example}"))

    assert spans.as_document(written) == _printed(SPANS_PROJECT, relative)


@pytest.mark.parametrize("relative", [relative for _, relative in EXAMPLES])
def test_the_report_satisfies_the_committed_schema(relative: str) -> None:
    jsonschema.validate(
        instance=_document(SPANS_PROJECT, relative),
        schema=_contract("schemas/spans.schema.json"),
    )


def test_every_span_picks_out_the_code_it_names() -> None:
    # The offsets are the whole product, and a table that counted characters
    # instead of bytes would be wrong only where the file is not ASCII.
    document = _document(SPANS_PROJECT, "src/billing/invoice.py")
    method = document["functions"][1]

    assert _bytes_of("src/billing/invoice.py", method["span"]).startswith(b"def total(self)")
    assert _bytes_of("src/billing/invoice.py", method["body_span"]).endswith(
        b"return self.base * (100 + rate) // 100"
    )
    assert _bytes_of("src/billing/invoice.py", method["excluded"][0]["span"]) == (
        '"""세금을 더한 합계를 돌려준다."""'.encode()
    )
    assert _bytes_of("src/billing/invoice.py", method["excluded"][1]["span"]) == b"int"


def test_a_decorated_method_reports_its_signature_outside_its_body() -> None:
    document = _document(SPANS_PROJECT, "src/billing/invoice.py")
    method = document["functions"][1]

    signature = _bytes_of("src/billing/invoice.py", {
        "start_byte": method["span"]["start_byte"],
        "end_byte": method["body_span"]["start_byte"],
    })

    # The decorator is before the span, and the return annotation is inside the
    # signature: neither needs a kind of its own to be out of a generator's reach.
    assert b"@cached_property" not in signature
    assert b"-> int" in signature
    assert b"@cached_property" not in _bytes_of("src/billing/invoice.py", method["span"])


def test_a_function_with_nothing_to_leave_alone_reports_no_exclusions() -> None:
    # The pairing rule: an empty list is left out, not written as `[]`.
    document = _document(SPANS_PROJECT, "src/scheduling/overlap.py")

    assert "excluded" not in document["functions"][0]


def test_a_nested_function_is_reported_after_the_one_that_holds_it() -> None:
    document = _document(SPANS_PROJECT, "src/scheduling/sync.py")

    outer, inner = document["functions"]
    assert (outer["qualified_name"], inner["qualified_name"]) == (
        "sync_events",
        "sync_events.<locals>.merge",
    )
    assert _bytes_of("src/scheduling/sync.py", outer["span"]).startswith(b"async def sync_events")
    assert outer["body_span"]["start_byte"] < inner["span"]["start_byte"]
    assert inner["span"]["end_byte"] <= outer["body_span"]["end_byte"]


def test_what_a_parent_owns_is_its_body_without_its_children() -> None:
    # The ownership rule the protocol documents, applied: the nested function is
    # not part of what a generator may aim at inside its parent.
    relative = "src/scheduling/sync.py"
    document = _document(SPANS_PROJECT, relative)
    outer, inner = document["functions"]

    before = _bytes_of(relative, {
        "start_byte": outer["body_span"]["start_byte"],
        "end_byte": inner["span"]["start_byte"],
    })
    after = _bytes_of(relative, {
        "start_byte": inner["span"]["end_byte"],
        "end_byte": outer["body_span"]["end_byte"],
    })

    assert b"def merge" not in after
    assert b"merged.update" not in after
    assert b"await asyncio.sleep(0)" in after
    # What is left of the parent's own opening is its docstring, which is excluded,
    # and the blank line after it.
    assert before.startswith('"""양쪽 예약을 하나로 맞춘다."""'.encode())
    assert outer["excluded"][0]["span"]["start_byte"] == outer["body_span"]["start_byte"]


def test_a_nested_function_owns_its_own_exclusions() -> None:
    document = _document(SPANS_PROJECT, "src/scheduling/sync.py")
    outer, inner = document["functions"]

    assert [excluded["kind"] for excluded in outer["excluded"]] == ["docstring"]
    assert [excluded["kind"] for excluded in inner["excluded"]] == ["annotation"]
    assert _bytes_of("src/scheduling/sync.py", inner["excluded"][0]["span"]) == b"dict[str, str]"


def test_the_file_hash_is_the_hash_of_the_bytes_that_were_read() -> None:
    relative = "src/scheduling/sync.py"
    document = _document(SPANS_PROJECT, relative)

    assert document["file_sha256"] == sha256((SPANS_PROJECT / relative).read_bytes()).hexdigest()
    assert document["file"] == relative


def test_a_file_without_functions_reports_an_empty_list(tmp_path: Path) -> None:
    (tmp_path / "constants.py").write_text("LIMIT = 60  # 한 시간\n", encoding="utf-8")

    assert _document(tmp_path, "constants.py")["functions"] == []


def test_a_function_defined_under_a_condition_is_still_reported(tmp_path: Path) -> None:
    # A function can stand anywhere a statement can, so the search cannot be a
    # walk over top-level statements only.
    (tmp_path / "conditional.py").write_text(
        "import sys\n\nif sys.version_info >= (3, 12):\n\n    def only_here() -> int:\n"
        "        return 1\n",
        encoding="utf-8",
    )

    document = _document(tmp_path, "conditional.py")

    assert [function["qualified_name"] for function in document["functions"]] == ["only_here"]


def test_a_class_inside_a_function_belongs_to_the_function(tmp_path: Path) -> None:
    (tmp_path / "inner.py").write_text(
        'def build() -> object:\n    class Held:\n        """안에 사는 클래스."""\n\n'
        "        limit: int = 3\n\n        def get(self) -> int:\n"
        "            return self.limit\n\n    return Held()\n",
        encoding="utf-8",
    )

    document = _document(tmp_path, "inner.py")

    assert [function["qualified_name"] for function in document["functions"]] == [
        "build",
        "build.<locals>.Held.get",
    ]
    # The class is the function's own to mutate, so what stands in it that carries
    # no behaviour is excluded against the function; its method is not, because the
    # method has an entry of its own.
    assert [excluded["kind"] for excluded in document["functions"][0]["excluded"]] == [
        "docstring",
        "annotation",
    ]


def test_the_report_is_the_last_line_even_when_the_file_is_large(tmp_path: Path) -> None:
    # One line, whatever the file's size: the protocol reserves the last line and
    # this document has no other place to go.
    source = "".join(f"def f{index}() -> int:\n    return {index}\n\n\n" for index in range(200))
    (tmp_path / "many.py").write_text(source, encoding="utf-8")

    completed = _spans(tmp_path, "many.py")

    assert (completed.returncode, completed.stderr) == (0, "")
    assert len(completed.stdout.splitlines()) == 1
    assert len(_last_line(completed.stdout)["functions"]) == 200


def test_a_byte_order_mark_is_refused(tmp_path: Path) -> None:
    (tmp_path / "marked.py").write_bytes(b"\xef\xbb\xbfdef f():\n    return 1\n")

    assert _refusal(tmp_path, "marked.py")["code"] == "target_has_byte_order_mark"


def test_a_file_that_is_not_utf8_is_refused(tmp_path: Path) -> None:
    (tmp_path / "latin.py").write_bytes(b"# \xe4\xf6\xfc\ndef f():\n    return 1\n")

    assert _refusal(tmp_path, "latin.py")["code"] == "target_not_utf8"


def test_a_file_with_carriage_returns_is_refused(tmp_path: Path) -> None:
    (tmp_path / "windows.py").write_bytes(b"def f():\r\n    return 1\r\n")

    assert _refusal(tmp_path, "windows.py")["code"] == "target_has_carriage_return"


@pytest.mark.parametrize(
    "cookie",
    [
        "# -*- coding: latin-1 -*-\n",
        "# coding=latin-1\n",
        "#!/usr/bin/env python\n# -*- coding: latin-1 -*-\n",
    ],
)
def test_a_file_declaring_another_encoding_is_refused(tmp_path: Path, cookie: str) -> None:
    # A file whose bytes happen to decode as UTF-8 while announcing something else
    # is a file the core refuses a run over, so a report of its spans would measure
    # offsets nothing would ever use them for.
    (tmp_path / "announced.py").write_text(f"{cookie}def f():\n    return 1\n", encoding="utf-8")

    assert _refusal(tmp_path, "announced.py")["code"] == "target_declares_other_encoding"


@pytest.mark.parametrize(
    "preamble",
    [
        # utf-8 spelled another way is still utf-8, and the core accepts it.
        "# -*- coding: UTF_8 -*-\n",
        # A declaration counts in the first two lines only; below them it is a
        # comment like any other, which is again where the core draws the line.
        "#!/usr/bin/env python\n# nothing to declare\n# -*- coding: latin-1 -*-\n",
        # Only a comment can declare an encoding, so the same words in a string
        # are just a string.
        'LABEL = "coding: latin-1"\n',
    ],
)
def test_a_file_the_core_would_accept_the_encoding_of_is_described(
    tmp_path: Path, preamble: str
) -> None:
    (tmp_path / "fine.py").write_text(f"{preamble}def f():\n    return 1\n", encoding="utf-8")

    document = _document(tmp_path, "fine.py")

    assert [function["qualified_name"] for function in document["functions"]] == ["f"]


def test_a_file_that_is_not_python_is_refused(tmp_path: Path) -> None:
    (tmp_path / "broken.py").write_text("def f(:\n", encoding="utf-8")

    assert _refusal(tmp_path, "broken.py")["code"] == "target_does_not_parse"


def test_a_missing_file_is_refused(tmp_path: Path) -> None:
    assert _refusal(tmp_path, "absent.py")["code"] == "target_missing"


def test_a_directory_is_refused(tmp_path: Path) -> None:
    (tmp_path / "package").mkdir()

    assert _refusal(tmp_path, "package")["code"] == "target_unreadable"


@pytest.mark.parametrize("spelling", ["../outside.py", "/etc/hosts", "src//module.py", ""])
def test_a_path_that_is_not_a_project_relative_spelling_is_refused(
    tmp_path: Path, spelling: str
) -> None:
    assert _refusal(tmp_path, spelling)["code"] == "invalid_target_path"


def test_a_file_that_is_a_link_out_of_the_project_is_refused(tmp_path: Path) -> None:
    project = tmp_path / "project"
    project.mkdir()
    outside = tmp_path / "elsewhere.py"
    outside.write_text("def f() -> int:\n    return 1\n", encoding="utf-8")
    (project / "linked.py").symlink_to(outside)

    assert _refusal(project, "linked.py")["code"] == "target_reached_through_link"


def test_a_file_reached_through_a_linked_directory_is_refused(tmp_path: Path) -> None:
    # The link is a directory on the way down, which no amount of reading the path
    # as text would reveal.
    project = tmp_path / "project"
    (project / "src").mkdir(parents=True)
    outside = tmp_path / "vendor"
    outside.mkdir()
    (outside / "module.py").write_text("def f() -> int:\n    return 1\n", encoding="utf-8")
    (project / "src" / "shared").symlink_to(outside)

    assert _refusal(project, "src/shared/module.py")["code"] == "target_reached_through_link"


def test_a_project_root_that_is_not_there_is_refused(tmp_path: Path) -> None:
    assert (
        _refusal(tmp_path / "nowhere", "module.py")["code"] == "project_root_unresolvable"
    )


def test_a_report_read_back_is_the_report_that_was_written() -> None:
    # The mirror is a contract model like any other: what the pack prints is what
    # it can read, which is what keeps a consumer in either language on the same
    # document.
    written = spans.report(SPANS_PROJECT, "src/billing/invoice.py")

    assert SpansReport.model_validate_json(spans.as_document(written)) == written
