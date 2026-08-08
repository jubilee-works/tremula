"""Where everything a run produces lives, and how it gets written.

One run means one directory, and its name is the run's identity: the core creates
`<parent>/<run-id>/` and hands the pack the whole path, so the pack reads the
identifier back off the basename rather than inventing a second one. That is also
what makes `collect` possible with nothing but a directory to go on.

Every file here is written by writing a neighbour and renaming it over the
target. A run that dies mid-write then leaves the previous version intact instead
of a truncated one — which matters most for `results.json`, the document a second
`collect` has to be able to reproduce.
"""

import os
import tempfile
from dataclasses import dataclass
from pathlib import Path

from tremula_python.contracts import Stage
from tremula_python.errors import PackFailure


@dataclass(frozen=True)
class RunLayout:
    """The paths of one run directory."""

    directory: Path

    @classmethod
    def at(cls, directory: Path) -> "RunLayout":
        """The layout of `directory`, resolved to an absolute path.

        Resolving is not cosmetic: subprocesses run with the project as their
        working directory, so a relative run directory would be written in the
        wrong place, and a `latest` symlink has to become the run it points at
        before its basename can be read as an identifier.

        Raises:
            PackFailure: The path has no basename to use as a run identifier —
                a filesystem root, or `.`.
        """
        resolved = directory.resolve()
        if not resolved.name:
            raise PackFailure(
                Stage.PREFLIGHT,
                "invalid_run_directory",
                f"`{directory}` cannot be a run directory: its last path segment is the "
                "run's identifier, and this path has none; pass a directory named after "
                "the run",
            )
        return cls(resolved)

    @property
    def run_id(self) -> str:
        """The run's identifier, which is the directory's name."""
        return self.directory.name

    @property
    def manifest(self) -> Path:
        """The manifest the run was started from, copied verbatim.

        `collect` is given nothing but a run directory, and it needs a mutant's
        span, original text, and replacement to report where the mutant landed
        and what it changed. This copy is where it reads them.
        """
        return self.directory / "manifest.json"

    @property
    def config(self) -> Path:
        """The generated backend configuration."""
        return self.directory / "config.toml"

    @property
    def session(self) -> Path:
        """The backend's own work database, kept for recovery and for reporting."""
        return self.directory / "session.sqlite"

    @property
    def targets(self) -> Path:
        """The list of files the runner hashes while a mutant is applied."""
        return self.directory / "targets.json"

    @property
    def expected_hashes(self) -> Path:
        """What those files must hash to: one per mutant, plus the unmutated ones."""
        return self.directory / "expected-hashes.json"

    @property
    def baseline(self) -> Path:
        """The unmutated reference run's document."""
        return self.directory / "baseline.json"

    @property
    def results(self) -> Path:
        """Everything the run observed, in neutral terms."""
        return self.directory / "results.json"

    @property
    def logs(self) -> Path:
        """Output kept for a human to read: the suite's, and the backend's."""
        return self.directory / "logs"

    @property
    def pycache(self) -> Path:
        """Where bytecode is cached, so no run reads or writes the project's own."""
        return self.directory / "pycache"

    def prepare(self) -> None:
        """Create the run directory and the subdirectories runs write into."""
        for directory in (self.directory, self.logs, self.pycache):
            directory.mkdir(parents=True, exist_ok=True)


def write_atomically(path: Path, text: str) -> None:
    """Write `text` to `path` by renaming a finished file over it."""
    handle, temporary = tempfile.mkstemp(dir=path.parent, prefix=f"{path.name}.", suffix=".part")
    try:
        with os.fdopen(handle, "w", encoding="utf-8") as stream:
            stream.write(text)
        os.replace(temporary, path)
    finally:
        # A successful rename leaves nothing behind; a failed write does.
        Path(temporary).unlink(missing_ok=True)
