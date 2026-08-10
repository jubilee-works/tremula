"""A sibling module, imported by its bare name.

Here so that a probe has to put the file's own directory on the search path: a
module that imports a neighbour this way is not importable from the project root
alone.
"""


def doubled(number: int) -> int:
    """Twice the number."""
    return number * 2
