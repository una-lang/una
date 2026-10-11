"""Operator families (docs/plan/security-hunt-generators.md §4). Each operator is a
function `(seed, edition) -> Program`, registered in `OPERATORS` by its id."""

from . import handlers, keys, trusted

OPERATORS = {}
OPERATORS.update(trusted.OPERATORS)
OPERATORS.update(handlers.OPERATORS)
OPERATORS.update(keys.OPERATORS)


def family_of(op):
    return op.rstrip("0123456789")
