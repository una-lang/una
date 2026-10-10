# sensors log

## check run 1
- U0101 "unexpected `=`, expected end of line" (assignment inside a `when` arm)
- consulted: message only
- enough: partly (it says an arm cannot hold an assignment, not what to write instead)
- changed: `answered = if a is Ok then answered + 1 else answered` instead of a `when` with assignment arms
- DOC: una-compact.md §Syntax/§Loops does not say whether a `when` arm can hold a statement (assignment); it cannot.

## check run 2
- no diagnostics, exit 0 (PASS)

## test run 1
- 3 examples passed, exit 0

## change: added a stand-in with two failing sensors and an example showing the earlier one in list order wins
