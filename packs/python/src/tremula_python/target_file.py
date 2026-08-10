"""Reading one of the project's own source files, and refusing anything else.

Two subcommands read a source file directly rather than through a manifest the
core has already validated — `spans`, which is asked where a mutation may land,
and `probe`, which is asked what one input does to a function. Neither has the
core's neutral checks standing behind it, so both perform the same ones here, and
they perform them in the core's own order so that the failure a caller is told
about is the one it would be told about with a manifest in hand.

The `stage` argument is what keeps the two apart in a failure report: the checks
are identical, and how far the pack had got when one of them refused is not.
"""

import ast
from itertools import takewhile
from pathlib import Path

from tremula_python.contracts import Stage
from tremula_python.errors import PackFailure

BYTE_ORDER_MARK = b"\xef\xbb\xbf"
"""What a UTF-8 encoder may prepend to a file, and what the core refuses."""

COOKIE_LINES = 2
"""How many leading lines may carry an encoding declaration, as the core counts them."""


def read_bytes(project_root: Path, relative: str, stage: Stage) -> bytes:
    """The raw bytes of `relative`, a file of the project at `project_root`.

    Raises:
        PackFailure: The path does not name a readable file of this project.
    """
    return read(canonical_root(project_root, stage), relative, stage)


def canonical_root(project_root: Path, stage: Stage) -> Path:
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
            stage,
            "project_root_unresolvable",
            f"the project root `{project_root}` cannot be resolved: {error}",
        ) from error
    if not resolved.is_dir():
        raise PackFailure(
            stage,
            "project_root_unresolvable",
            f"the project root `{project_root}` is not a directory",
        )
    return resolved


def project_file(root: Path, relative: str, stage: Stage) -> Path:
    """The file `relative` names, refused unless it is really the project's own.

    The spelling is checked as text first, because a host's own path syntax is
    not the contract's: on this one `..\\escape.py` is a single ordinary name.
    Then every step of the way down is checked for being a link, because a
    relative path with no `..` in it still leaves the project when the filesystem
    cooperates — and reading a file the project does not own means answering about
    somebody else's code.

    Raises:
        PackFailure: The path is not a plain project-relative POSIX path, leads
            through a link, or lands outside the project.
    """
    names = relative.split("/")
    if not _is_plain_posix_spelling(relative) or any(
        name in {"", ".", ".."} for name in names
    ):
        raise PackFailure(
            stage,
            "invalid_target_path",
            f"`{relative}` is not a project-relative POSIX path; spell the file the "
            "way a manifest would, as `src/module.py`",
        )
    path = root
    for name in names:
        path = path / name
        if path.is_symlink():
            raise PackFailure(
                stage,
                "target_reached_through_link",
                f"`{relative}` reaches the file through the link `{name}`; describe the "
                "file where it really lives",
            )
    # The walk above has ruled out every spelling that leaves the project, so this
    # is the check that does not depend on that enumeration being complete.
    if not path.resolve().is_relative_to(root):
        raise PackFailure(
            stage,
            "target_outside_project",
            f"`{relative}` lands outside the project at `{root}`",
        )
    return path


def read(root: Path, relative: str, stage: Stage) -> bytes:
    """Read the project's file, keeping "not there" apart from "cannot read it".

    Raises:
        PackFailure: The path is not the project's, the file is missing, or it is
            there and unreadable.
    """
    path = project_file(root, relative, stage)
    try:
        return path.read_bytes()
    except FileNotFoundError as error:
        raise PackFailure(
            stage,
            "target_missing",
            f"`{relative}` is not a file of this project",
        ) from error
    except OSError as error:
        raise PackFailure(
            stage,
            "target_unreadable",
            f"`{relative}` cannot be read: {error}",
        ) from error


def decoded(source: bytes, relative: str, stage: Stage) -> str:
    """The file as text, refused unless it is the plain UTF-8 the contract assumes.

    Each refusal here is one the core's own validation would reach for the same
    file, and they are taken in the core's order. Answering about a file the core
    refuses would be answering about offsets no run could ever use.

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
            stage,
            "target_has_byte_order_mark",
            f"`{relative}` starts with a byte-order mark; save it as plain UTF-8",
        )
    try:
        text = source.decode("utf-8")
    except UnicodeDecodeError as error:
        raise PackFailure(
            stage,
            "target_not_utf8",
            f"`{relative}` is not UTF-8: {error}",
        ) from error
    if _declares_other_encoding(text):
        raise PackFailure(
            stage,
            "target_declares_other_encoding",
            f"`{relative}` declares an encoding other than UTF-8; only plain UTF-8 "
            "sources are supported",
        )
    if "\r" in text:
        raise PackFailure(
            stage,
            "target_has_carriage_return",
            f"`{relative}` uses carriage returns; tremula works on files with "
            "line-feed endings only",
        )
    return text


def parsed(text: str, relative: str, stage: Stage) -> ast.Module:
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
            stage,
            "target_does_not_parse",
            f"`{relative}` is not valid Python: {error}",
        ) from error


def _is_plain_posix_spelling(relative: str) -> bool:
    """Whether the text is a non-empty relative POSIX path with no empty segment."""
    return bool(relative) and not (
        "\\" in relative
        or relative.startswith("/")
        or relative.endswith("/")
        or "//" in relative
    )


def _declares_other_encoding(text: str) -> bool:
    """Whether a leading comment declares an encoding that is not UTF-8.

    The reading is the core's, step for step, because being stricter here would
    refuse a file a run would go on to accept and being laxer would answer about
    one it would not: the first two lines only, comments only, `coding` followed by
    a colon or an equals sign, and the name folded so that `UTF-8`, `utf_8`, and
    `utf8` are one name. Lines are split on the line feed alone, so the other
    characters Python calls line breaks do not open a third line the core would not
    have looked at.
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
