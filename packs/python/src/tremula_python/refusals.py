"""Which mutants a run refused before it could try them, and why.

A manifest the core has accepted can still hold a mutant this language cannot
apply, and there are three points at which that becomes apparent: the language
checks, the session file the backend reads its parameters out of, and the
backend's own count of the jobs it made from them. Any one of them used to end
the run, which meant no results document at all — so one impossible mutant cost
every other mutant in the batch its verdict.

The run now leaves that mutant out and records it here. `collect` reads the file
back and reports each named mutant as a mutation that was never applied, carrying
the reason, because nothing else in the run directory has anywhere to keep it: the
session holds jobs, and a refused mutant has none.

The record is written once, before anything is executed, and read by a `collect`
that may be running long afterwards over an interrupted run. A run directory
without the file refused nothing a reader can name — which is also true of every
run directory made before this file existed.
"""

import json
from dataclasses import dataclass
from typing import cast

from tremula_python.run_layout import RunLayout, write_atomically


@dataclass(frozen=True)
class Refusal:
    """Why one mutant was left out of the run.

    The same pair a pack failure carries: a stable code for a reader that
    branches, and a message for one that reads.
    """

    code: str
    """Stable identifier for this kind of refusal, such as `unserializable_mutant`."""

    message: str
    """What happened, in words, naming the mutant it happened to."""


Refused = dict[str, Refusal]
"""Every refused mutant's reason, by mutant identifier."""


def write(layout: RunLayout, refused: Refused) -> None:
    """Record which mutants the run left out.

    Written even when nothing was refused, so that the file's presence means the
    run got far enough to know rather than that something went wrong.
    """
    document = {
        mutant_id: {"code": refusal.code, "message": refusal.message}
        for mutant_id, refusal in refused.items()
    }
    write_atomically(layout.refusals, json.dumps(document))


def read(layout: RunLayout) -> Refused:
    """Read the record back, or nothing when the run kept none.

    Raises:
        ValueError: The file is there and is not a record of refusals. It is read
            straight into the results document, so a document of some other shape
            would put whatever it holds in front of the core.
    """
    if not layout.refusals.is_file():
        return {}
    document: object = json.loads(layout.refusals.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise _not_a_record()
    refused: Refused = {}
    for mutant_id, entry in cast("dict[object, object]", document).items():
        if not isinstance(mutant_id, str) or not isinstance(entry, dict):
            raise _not_a_record()
        fields = cast("dict[str, object]", entry)
        code = fields.get("code")
        message = fields.get("message")
        if not isinstance(code, str) or not isinstance(message, str):
            raise _not_a_record()
        refused[mutant_id] = Refusal(code, message)
    return refused


def _not_a_record() -> ValueError:
    return ValueError(
        "refusals.json is not a record of refusals: it has to be an object mapping each "
        "mutant identifier to an object with a `code` and a `message`"
    )
