"""What this pack tells the core about itself.

The core calls `--capabilities` before it calls anything else and compares the
answer against the ranges it supports. Everything here is therefore a claim the
rest of the pack has to keep: the subcommands really are implemented, and every
name in `validate_checks` really is a check `validate` performs.
"""

from importlib.metadata import version

from tremula_python.contracts import CONTRACT_VERSION, Capabilities, PackInfo

DISTRIBUTION = "tremula-python"
"""The pack's distribution name, which is also how the core refers to it."""

SUBCOMMANDS = ("run", "collect", "validate")
"""The work subcommands. `--capabilities` is the handshake, not one of them."""

VALIDATE_CHECKS = ("parses", "single_statement", "round_trips", "span_matches_node")
"""The language-level checks `validate` performs, in the order it applies them.

The first three come from the replacement itself: it is syntactically valid
Python, it is a single top-level statement, and it survives being parsed and
injected without losing text. The fourth is about the target: the span lines up
with a node the backend can match.
"""


def build() -> Capabilities:
    """Describe this pack."""
    info = pack_info()
    return Capabilities(
        name=info.name,
        version=info.version,
        contract_version=info.contract_version,
        subcommands=list(SUBCOMMANDS),
        validate_checks=list(VALIDATE_CHECKS),
    )


def pack_info() -> PackInfo:
    """This pack's provenance, as the documents it writes carry it.

    The version comes from installed distribution metadata rather than a literal,
    because the pack is only usable when it is installed — Cosmic Ray finds its
    operator through an entry point, which exists only in that metadata.
    """
    return PackInfo(
        name=DISTRIBUTION,
        version=version(DISTRIBUTION),
        contract_version=CONTRACT_VERSION,
    )
