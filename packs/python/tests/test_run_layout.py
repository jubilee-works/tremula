"""The run directory: what counts as one, and how files land in it.

A run's identity is its directory's name, so a path with no name of its own is
refused rather than quietly resolved into whatever directory the pack happens to
be standing in — which would scatter a run's artifacts across a working tree.
"""

from pathlib import Path

import pytest

from tremula_python.errors import PackFailure
from tremula_python.run_layout import RunLayout, write_atomically


@pytest.mark.parametrize("spelling", ["", ".", "..", "/", "runs/..", "./"])
def test_a_path_with_no_name_of_its_own_is_refused(spelling: str) -> None:
    # Checked before the path is resolved, because resolving hides the problem:
    # `.` becomes the working directory, which has a perfectly good name that
    # says nothing about any run.
    with pytest.raises(PackFailure) as raised:
        RunLayout.at(Path(spelling))

    assert (raised.value.stage.value, raised.value.code) == (
        "preflight",
        "invalid_run_directory",
    )


def test_the_run_identifier_is_the_directorys_name(tmp_path: Path) -> None:
    layout = RunLayout.at(tmp_path / "20260808T120000Z-3b1f8c")

    assert layout.run_id == "20260808T120000Z-3b1f8c"


def test_a_relative_path_becomes_absolute(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    # Subprocesses run with the project as their working directory, so a relative
    # run directory would be written somewhere else entirely.
    monkeypatch.chdir(tmp_path)

    layout = RunLayout.at(Path("run-1"))

    assert layout.directory == tmp_path.resolve() / "run-1"


def test_a_symlink_is_read_as_the_run_it_points_at(tmp_path: Path) -> None:
    # `latest` is how a run directory is found without knowing its name; the
    # identifier still has to be the real run's.
    real = tmp_path / "20260808T120000Z-3b1f8c"
    real.mkdir()
    link = tmp_path / "latest"
    link.symlink_to(real)

    assert RunLayout.at(link).run_id == real.name


def test_every_artifact_lives_inside_the_run_directory(tmp_path: Path) -> None:
    layout = RunLayout.at(tmp_path / "run-1")

    for path in (
        layout.manifest,
        layout.config,
        layout.session,
        layout.targets,
        layout.expected_hashes,
        layout.diffs,
        layout.baseline,
        layout.results,
        layout.logs,
        layout.pycache,
    ):
        assert path.parent == layout.directory


def test_a_written_file_leaves_no_half_written_neighbour(tmp_path: Path) -> None:
    layout = RunLayout.at(tmp_path / "run-1")
    layout.prepare()

    write_atomically(layout.results, "first")
    write_atomically(layout.results, "second")

    assert layout.results.read_text(encoding="utf-8") == "second"
    assert sorted(path.name for path in layout.directory.iterdir()) == [
        "logs",
        "pycache",
        "results.json",
    ]
