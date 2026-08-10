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

SUBCOMMANDS = ("run", "collect", "validate", "spans")
"""The work subcommands. `--capabilities` is the handshake, not one of them.

`spans` is last because it arrived last, and because a core that has never heard
of it still works with this pack: the handshake asks for the subcommands the core
needs, not for the ones the pack has.
"""

VALIDATE_CHECKS = (
    "compiles_in_file",
    "single_statement",
    "round_trips",
    "span_matches_node",
    "ast_equal",
)
"""The language-level checks `validate` performs, in the order it applies them.

`compiles_in_file` is the target file with the replacement spliced into the span,
compiled whole. It replaced a check that read the replacement on its own, which
refused a `return` statement for not being a module — 34 such refusals in a
measured 144, every one of which compiled where it belonged. The next two come
from the replacement itself: it is a single top-level statement, and it survives
being parsed and injected without losing text. The fourth is about the target:
the span lines up with a node the backend can match. The last is about the pair:
`ast_equal` refuses a mutation whose file has the syntax tree the original had,
which is a mutant no test suite could be blamed for missing.
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
