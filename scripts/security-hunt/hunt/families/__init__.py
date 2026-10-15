"""Operator families (docs/plan/security-hunt-generators.md §4). Each operator is a
function `(seed, edition) -> Program`, registered in `OPERATORS` by its id."""

from . import (boundaries, composed, composed2, concurrency, conditions, fragment, generics,
               handlers, keys, ordering, payloads, sums, trusted)

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
OPERATORS.update(boundaries.OPERATORS)
OPERATORS.update(conditions.OPERATORS)

# The editions of the fixed run, by operator: 0.1 and 0.2 unless the operator's family says
# otherwise. An operator of an edition 0.3 rule runs 0.2, as the edition gate's probe, and 0.3.
DEFAULT_EDITIONS = ("0.1", "0.2")
OPERATOR_EDITIONS = {**boundaries.EDITIONS, **conditions.EDITIONS}


def editions_of(op):
    return OPERATOR_EDITIONS.get(op, DEFAULT_EDITIONS)


def family_of(op):
    return op.rstrip("0123456789")
