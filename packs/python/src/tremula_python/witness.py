"""What a witness may say, which is a list of syntax and nothing else.

A witness is text a model wrote, and the whole of what is done with it is to read
one call out of it: the name of a function, and arguments that are literals. That
reading is written here as an explicit list of the syntax it accepts, rather than
as a delegation to `ast.literal_eval`.

Two reasons for the list to be spelled out rather than borrowed.

**`literal_eval` accepts more than literals.** `set()` passes it — an empty set has
no literal spelling, so that function makes an exception for the one call that
builds one. The exception is a sensible thing for a reader of data to do and the
wrong thing for a rule about what a model may put in front of an interpreter; and a
rule that is a side effect of another function's implementation is one that changes
when the standard library changes, without anybody deciding it.

**A witness has a size.** A hundred thousand elements is a literal too, and neither
reading one nor rendering what a function did with it is work a single survivor is
worth. [`MAX_NODES`] is the whole of what one may be.

Nothing here executes anything. A node that passes the list is turned into a value
with `ast.literal_eval`, which for these nodes builds and does not evaluate.
"""

import ast
from typing import Any

MAX_NODES = 500
"""How many pieces of syntax one whole call may be, its own name included.

Generous for what a witness is — `f(1, 2, {"a": [3]})` is a dozen — and small
enough that refusing an enormous one is cheaper than reading it. Counted in syntax
rather than in characters, because what a large witness costs is the walking.
"""

_CONSTANTS = (bool, int, float, str, bytes)
"""What a constant in an argument may be, besides `None`.

A complex number and an ellipsis are constants to the parser and are not on this
list: neither is a value a witness has any business naming, and both would arrive
in a rendering nobody reads twice.
"""


def one_call(text: str) -> tuple[str, ast.Call] | None:
    """The name and the syntax of one call of a plain name, or None if that is not it.

    An attribute, a name from the module, a call inside the call, an unpacking, or
    two expressions in a row is each a way for something to have to run before the
    function is reached, and running what a model wrote is the one thing refused.
    """
    try:
        expression = ast.parse(text, mode="eval").body
    except (SyntaxError, ValueError, MemoryError, RecursionError):
        return None
    if not isinstance(expression, ast.Call) or not isinstance(expression.func, ast.Name):
        return None
    if any(keyword.arg is None for keyword in expression.keywords):
        return None
    if any(isinstance(argument, ast.Starred) for argument in expression.args):
        return None
    return expression.func.id, expression


def arguments_of(call: ast.Call) -> tuple[list[Any], dict[str, Any]] | None:
    """What the call's arguments are as values, or None if they are not to be read.

    None for a call of more syntax than [`MAX_NODES`], and for one holding any node
    this module's list does not name.
    """
    if sum(1 for _ in ast.walk(call)) > MAX_NODES:
        return None
    given = [*call.args, *(keyword.value for keyword in call.keywords)]
    if not all(_allowed(argument) for argument in given):
        return None
    try:
        positional = [ast.literal_eval(argument) for argument in call.args]
        named = {
            str(keyword.arg): ast.literal_eval(keyword.value)
            for keyword in call.keywords
            if keyword.arg is not None
        }
    except (ValueError, SyntaxError, TypeError, MemoryError, RecursionError):
        return None
    return positional, named


def function_named(text: str) -> str | None:
    """The function a witness calls, or None when the whole of it is not one to run."""
    read = one_call(text)
    if read is None:
        return None
    name, call = read
    return None if arguments_of(call) is None else name


def _allowed(node: ast.expr) -> bool:
    """Whether one piece of an argument is a shape on the list.

    A constant, a list, tuple, set or dict built out of shapes on the list, or a
    number with a leading `-`. Everything else is refused, and refusing is the
    default: a node this does not recognise is not allowed by omission.
    """
    if isinstance(node, ast.Constant):
        return node.value is None or isinstance(node.value, _CONSTANTS)
    if isinstance(node, (ast.List, ast.Tuple, ast.Set)):
        return all(_allowed(item) for item in node.elts)
    if isinstance(node, ast.Dict):
        keyed = all(key is not None and _allowed(key) for key in node.keys)
        return keyed and all(_allowed(value) for value in node.values)
    if isinstance(node, ast.UnaryOp):
        return isinstance(node.op, ast.USub) and _number(node.operand)
    return False


def _number(node: ast.expr) -> bool:
    """Whether the node is a plain number, which is what a leading `-` may lead."""
    return (
        isinstance(node, ast.Constant)
        and isinstance(node.value, (int, float))
        and not isinstance(node.value, bool)
    )
