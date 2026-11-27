# Security policy

Una makes promises about programs: that they use no authority beyond what their headers declare, that labeled data does not flow where its labels forbid, and that proven contracts hold. A way to break one of those promises is a security problem, and we want to hear about it.

## Reporting a vulnerability

Please report vulnerabilities privately, not in a public issue. Use any of these channels:

- **GitHub private vulnerability reporting**: on the repository's [Security tab](https://github.com/una-lang/una/security), choose "Report a vulnerability".
- **Email**: <contact+security@una-lang.org>.

Include:
- the Una version (`una --version`) and your platform;
- the smallest program or input that shows the problem;
- the commands you ran;
- what happened, and what Una promised instead.

We will acknowledge your report, keep you informed while we work on it, and thank you by name in the release notes unless you prefer otherwise. Please give us a reasonable time to release a fix before you disclose the problem publicly.

## Supported versions

Una is a research language and is not ready for production use; edition 0.4, first shipped by v0.4.0, is still a preview in v0.5.2 (ADR 0121). Fixes go into the next release. Past fixes were also released as patches of the lines v0.3.x (branch `release/v0.3`) and v0.1.x (branch `release/v0.1`): each advisory in [`docs/security/advisories.md`](docs/security/advisories.md) names the releases that fix it, and says when a line is not fixed. A version is fixed only when an advisory names a release that carries the fix.

## Scope

In scope:

- **The checker's soundness.** `una check` accepts a program that:
  - uses authority its header does not declare;
  - lets labeled data reach a sink its label forbids, other than through a declared policy;
  - violates a contract or refinement that the verifier reports as proven.
  This includes errors in how obligations are encoded for the solver, and the proof cache returning a result that does not belong to the query.
- **The runtime.**
  - The interpreter, or a native executable, gives a program authority beyond its grants.
  - Behavior differs between the interpreter and the native runtime in a way that breaks a guarantee.
  - Memory unsafety in the native runtime.
- **The trusted base, as the build report lists it.** For example:
  - a host capability that acts outside its grant;
  - a standard library function that breaks its specification;
  - the installer accepting an archive or a solver whose digest does not match its record.

Out of scope:

- What Una states as outside its model: timing, termination and resource side channels, and external systems that behave differently from their declared signatures.
- Code in a `trusted` module or a declassification policy that does what it says: these are trusted by design, and the build report lists them.
- Bugs in Z3 itself. Report those to the [Z3 project](https://github.com/Z3Prover/z3), and tell us if Una depends on the behavior.

A soundness bug that does not put anyone at risk can be reported in public with the soundness report template. If you are unsure, report privately.
