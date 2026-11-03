# Draft amendment to ADR 0117, rule 2 (Pinning): the seen value of the attention word

Status: draft, for the project to accept as a dated amendment to ADR 0117. The accepted ADR text
is not edited. Card #1533.

## Text proposed for the amendment

Rule 2, Pinning, lists a "copy of the attention word" among the slots a switch swaps. The
amendment corrects this. A switch does not swap the attention word, or the value its last check
point saw. Both belong to the worker thread, and a task's install keeps the seen value. The
slicer raises a worker's word when the turn in progress reaches its end, whichever task holds
that turn. So a raise that is pending when the worker passes to another task, or when that task
installs its state, stays pending until that task's next check point.

## Why

A worker sets a turn's end when it takes the job, before the task installs its state. If the
install took the word's value as seen, a raise that came in between was lost. The slicer
raises each end only once, so the task kept its worker. Then a ready task's wait was not
bounded by the slices ahead of it, which breaks the Fairness bound of rule 2. The cost is one
slow path at most per task start, and only when a raise is pending. Under virtual time the seen
value is not read, so traces and stop schedules do not change.
