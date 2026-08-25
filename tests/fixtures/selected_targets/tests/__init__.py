"""Makes the suite a package, so its imports resolve from the project root.

Without this, pytest puts `tests/` itself on the import path and `from ranges
import overlaps` finds nothing. It is also what the convention this fixture is
here to exercise expects: the tests of a package live under the package root,
not beside the module.
"""
