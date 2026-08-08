"""The pack's failure channel: one exception, rendered as one document.

Every way the pack can stop early raises `PackFailure`. The entry point catches
it and prints the `pack-error` document it carries as the last line of stdout,
which is the only failure report the core is contracted to read — an execution
backend may discard stderr, so a traceback there would leave the core with an
exit code and nothing else.
"""

from collections.abc import Generator
from contextlib import contextmanager

from tremula_python.contracts import PackError, PackErrorDetail, Stage

UNEXPECTED_ERROR = "unexpected_error"
"""Code for a failure the pack has no specific diagnosis for."""


class PackFailure(Exception):
    """A diagnosed failure: the step that failed, a stable code, and a message.

    The three fields are the `pack-error` contract. `code` is what the core
    branches on and outlives any rewording of `message`.
    """

    def __init__(self, stage: Stage, code: str, message: str) -> None:
        """Record which step failed, why, and how to say so to a human."""
        super().__init__(message)
        self.stage = stage
        self.code = code
        self.message = message

    @property
    def document(self) -> PackError:
        """This failure as the document the core parses."""
        return PackError(
            error=PackErrorDetail(stage=self.stage, code=self.code, message=self.message)
        )


def unexpected(stage: Stage, error: BaseException) -> PackFailure:
    """Turn an exception nobody planned for into a reportable failure.

    The message is the exception's type and text rather than a traceback: it
    travels in a JSON document a machine parses, and the type is what makes an
    unhandled `FileNotFoundError` distinguishable from an unhandled `KeyError`
    in a bug report.
    """
    return PackFailure(stage, UNEXPECTED_ERROR, f"{type(error).__name__}: {error}")


@contextmanager
def failures_as(stage: Stage) -> Generator[None, None, None]:
    """Attribute whatever the block raises to `stage`.

    Diagnosed failures pass through untouched — they already name the step they
    belong to. Anything else becomes an `unexpected_error` for this stage, so the
    core still learns how far the pack got.
    """
    try:
        yield
    except PackFailure:
        raise
    except Exception as error:
        raise unexpected(stage, error) from error
