"""Operator families (docs/plan/security-hunt-generators.md §4). Each operator is a
function `(seed, edition) -> Program`, registered in `OPERATORS` by its id."""

from . import (boundaries, composed, composed2, concurrency, conditions, dependencies, fragment,
               generics, handlers, keys, ordering, payloads, registry, sums, timing, trusted)

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
OPERATORS.update(dependencies.OPERATORS)
OPERATORS.update(registry.OPERATORS)

# Family W, tasks and time (#973), runs under the cost clock in `hunt timing` alone, never in the
# release gate's fixed run, which runs every operator of `OPERATORS`: so it is kept apart.
TIMING_OPERATORS = dict(timing.OPERATORS)

# The editions of the fixed run, by operator: 0.1 and 0.2 unless the operator's family says
# otherwise. An operator of an edition 0.3 rule runs 0.2, as the edition gate's probe, and 0.3;
# one of edition 0.4's packages (families D and Y) runs 0.3, as the probe, and 0.4.
DEFAULT_EDITIONS = ("0.1", "0.2")
OPERATOR_EDITIONS = {**boundaries.EDITIONS, **conditions.EDITIONS, **dependencies.EDITIONS_BY_OP,
                     **registry.EDITIONS_BY_OP}


def editions_of(op):
    return OPERATOR_EDITIONS.get(op, DEFAULT_EDITIONS)


def family_of(op):
    return op.rstrip("0123456789")


# The edition 0.3 sample of every operator that does not run edition 0.3 (#818): some of its
# programs, by seed, also run in edition 0.3, where its rules are on edition-gated code paths. The
# seeds are those of its other editions, so every other program of the run stays the same.
SAMPLE_EDITION = "0.3"
SAMPLE_COUNT = 10
# The edition whose programs' forms the sample covers: a sample program is its program of this
# edition with the same seed.
TWIN_EDITION = "0.2"


# How far past the fixed run's seeds the sample looks for a seed whose edition 0.2 program has a
# form that only the operator's programs of another edition have (`sample_forms`).
SAMPLE_SEARCH = 1000


def sample_forms(op, seed_of, count):
    """The sample of `op`, whose `i`-th seed is `seed_of(i)` and whose fixed run has `count`
    seeds: the indices it runs in edition 0.3, and the forms it cannot cover. The indices are the
    first `SAMPLE_COUNT`, the first of each further form (variant) of its edition 0.2 programs,
    and, for each form its programs of another edition have but its edition 0.2 programs do not,
    the first index past `count`, within `SAMPLE_SEARCH`, whose edition 0.2 program has it. So
    every form the fixed run gives the operator, in any edition, also runs in 0.3; a form not
    found is returned as not covered. Both are empty for an operator that runs edition 0.3
    already."""
    if SAMPLE_EDITION in editions_of(op):
        return set(), set()
    chosen, forms, other = set(), set(), set()
    for i in range(count):
        form = OPERATORS[op](seed_of(i), TWIN_EDITION).variant
        if i < SAMPLE_COUNT or form not in forms:
            chosen.add(i)
            forms.add(form)
        other |= {OPERATORS[op](seed_of(i), ed).variant
                  for ed in editions_of(op) if ed != TWIN_EDITION}
    missing = other - forms
    for i in range(count, count + SAMPLE_SEARCH):
        if not missing:
            break
        form = OPERATORS[op](seed_of(i), TWIN_EDITION).variant
        if form in missing:
            chosen.add(i)
            missing.discard(form)
    return chosen, missing


def sample_indices(op, seed_of, count):
    """The indices of the programs of `op` that the sample runs in edition 0.3 (`sample_forms`);
    those from `count` on run in edition 0.3 only."""
    return sample_forms(op, seed_of, count)[0]


def editions_for(op, i, sample=None):
    """The editions of the `i`-th program of the fixed run of `op`: its editions, and the sample's
    when `i` is in `sample` (`sample_indices`), or, without one, among the first `SAMPLE_COUNT`."""
    editions = editions_of(op)
    picked = i < SAMPLE_COUNT if sample is None else i in sample
    if SAMPLE_EDITION in editions or not picked:
        return editions
    return editions + (SAMPLE_EDITION,)
