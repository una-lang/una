---
name: Soundness report
about: una check accepts a program that breaks a guarantee Una claims to prove
title: "Soundness: "
labels: ["bug"]
assignees: []
---

<!-- A soundness bug is the most valuable report you can send. If it lets a program escape its grants at run time or leak labeled data in a way that could hurt users, report it privately as SECURITY.md describes, not here. -->

## The guarantee that breaks

<!-- Pick one and delete the others. -->

- [ ] Types or refinements
- [ ] Authority: a capability used without being declared or granted
- [ ] Information flow: labeled data reaching a sink its label forbids
- [ ] Contracts: a `requires` or `ensures` reported as proven that does not hold
- [ ] The interpreter and a native executable disagree
- [ ] Other:

## The program

```una

```

<!-- If the package matters, include una.toml as well. -->

## Evidence

<!-- What `una check` reports (or does not report), and a run, test or argument that shows the guarantee is broken, for example the values that violate an ensures clause. -->

## Environment

- `una --version`:
- Platform:
