"""Operator families (docs/plan/security-hunt-generators.md §4). Each operator is a
function `(seed, edition) -> Program`, registered in `OPERATORS` by its id."""

from . import composed, composed2, concurrency, fragment, generics, handlers, keys, ordering, payloads, sums, trusted

OPERATORS = {}
OPERATORS.update(trusted.OPERATORS)
OPERATORS.update(handlers.OPERATORS)
OPERATORS.update(keys.OPERATORS)
OPERATORS.update(payloads.OPERATORS)
OPERATORS.update(generics.OPERATORS)
OPERATORS.update(concurrency.OPERATORS)
OPERATORS.update(sums.OPERATORS)
OPERATORS.update(ordering.OPERATORS)
OPERATORS.update(fragment.OPERATORS)
OPERATORS.update(composed.OPERATORS)
OPERATORS.update(composed2.OPERATORS)


def family_of(op):
    return op.rstrip("0123456789")
