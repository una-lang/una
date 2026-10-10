"""Operator families (docs/plan/security-hunt-generators.md §4). Each operator is a
function `(seed, edition) -> Program`, registered in `OPERATORS` by its id."""

from . import trusted

OPERATORS = {}
OPERATORS.update(trusted.OPERATORS)


def family_of(op):
    return op.rstrip("0123456789")
