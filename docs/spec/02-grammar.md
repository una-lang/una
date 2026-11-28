# 02. Grammar

Status: Normative, v0.1.

Notation: EBNF. `{ x }` is zero or more, `[ x ]` is optional, `|` separates alternatives, terminals are quoted. `NEWLINE`, `INDENT`, `DEDENT`, `lower_ident`, `upper_ident`, `cap_ref`, `int_lit` and `text_lit` come from `spec/01`. Doc comments (`---`) are attached by the lexer to the following declaration and do not appear in the grammar.

The grammar is LL(2) with one exception: a `kind` body is a sum if its right-hand side contains `|` at bracket depth zero (§2), which the parser decides by scanning to the end of the logical line.

(v0.2, ADR 0056.) `spec/17 §1` extends this grammar with:
- parameter conventions and call-site markers (`edit`, `take`);
- `let edit` bindings, assignments to places and `for` loops;
- `if` without `else` as a statement;
- `old` in `ensures`.

## 1. Files and modules

```ebnf
file            = module_header { import } { decl } ;
module_header   = [ "trusted" ] "module" module_path NEWLINE ;
module_path     = lower_ident { "." lower_ident } ;
import          = "import" module_path "(" import_name { "," import_name } ")" NEWLINE ;
import_name     = lower_ident | upper_ident ;

decl            = kind_decl | predicate_decl | function_decl | capability_decl
                | handler_decl | policy_decl | requirement_decl | example_decl
                | test_decl ;                                (* edition 0.3, ADR 0105 *)
```

(v0.3, ADR 0104.) In edition `0.3`, `decl` also takes `measure_decl`, and a type takes a unit after its core, `type = [ label ] type_core [ "of" unit_expr ] [ "in" range ]` (`spec/25 §1`, `§2`).

(v0.5, ADR 0126 rules 1 and 9.) In edition `0.5`, `decl` also takes `constant_decl`, a module constant:

```ebnf
constant_decl   = [ "pub" ] "const" lower_ident ":" type "=" line_expr NEWLINE ;
```

The type is required: without it, the declaration is U0101 at `=`. The value is a constant expression (`spec/10 §8`), one logical line, continued inside brackets as any line is; a doc comment may lead it. A `primary` that is a `lower_ident` may resolve to a constant (§7). In editions `0.1` to `0.4`, `const` and `let` at the top level are U0101; in edition `0.5`, `let` at the top level is U7007 with the exact rewrite to `const` (`spec/01 §5`), and is read as `const`.

Declarations within a module may appear in any order; all top-level names of a module are visible throughout it.

## 2. Kinds and types

```ebnf
kind_decl       = [ "pub" ] "kind" upper_ident [ type_params ] kind_body ;
type_params     = "[" upper_ident { "," upper_ident } "]" ;
kind_body       = alias_body | sum_body | record_body ;
alias_body      = "=" type [ "where" line_expr ] NEWLINE ;
sum_body        = "=" ( "|" variant { "|" variant }
                      | variant "|" variant { "|" variant } ) NEWLINE ;
record_body     = NEWLINE INDENT field { field } DEDENT ;
field           = lower_ident ":" type NEWLINE ;
variant         = upper_ident [ "(" field_sig { "," field_sig } ")" ] ;
field_sig       = lower_ident ":" type ;

type            = [ label ] type_core [ "in" range ] ;
label           = level [ "[" upper_ident { "," upper_ident } "]" ] ;
level           = "public" | "internal" | "confidential" | "secret" | "restricted" ;
type_core       = upper_ident [ "[" type { "," type } "]" ]
                | "[" type "]"
                | "fn" "(" [ type { "," type } ] ")" "->" type { fn_type_clause } ;
fn_type_clause  = "uses" cap_ref ":" cap_type | "fails" upper_ident ;
cap_type        = upper_ident [ "[" lower_ident { "," lower_ident } "]" ] ;
range           = signed_int ( ".." | "..<" ) signed_int ;
signed_int      = [ "-" ] int_lit ;
```

(v0.3, ADR 0092.) In edition `0.3`, the `uses` of a function's header and of a handler take a `cap_type` whose operations may carry a condition:

```ebnf
cap_type_cond   = upper_ident [ "[" cap_op { "," cap_op } "]" ] ;
cap_op          = lower_ident [ "gives" label ] [ "where" line_expr ] ;    (* "gives": ADR 0094 *)
```

(v0.3, ADR 0094 rule 1.) This grammar is the one that ADR 0094's Amends list names `spec/02 §3`; it lives here, in §2. `gives` and a label, the `label` of `spec/07 §2`, come before the condition: `owner gives internal[Service] where not dry_run`. In a function type's `uses`, `gives` is U0101 in every edition; in a header's `uses` before edition `0.3` it is U0103 (`spec/01 §5`).

A `cap_op` ends at the first `,` or `]` at bracket and parenthesis depth 0: every nested `,`, `[ ]` or `( )` belongs to the expression. A function type's `uses` (`fn_type_clause`) keep `cap_type`, so `where` there is U0101 in every edition, as it is in a header's `uses` in editions `0.1` and `0.2`. The condition's meaning is in `spec/05 §3`.

After a `level`, `[` always starts the label's categories. A labeled list type is written with the named form, `secret List[Int]`, or after the categories, `secret[PII] [Int]`. A label with categories that is not followed by a type is U0101 (ADR 0018).

(v0.5, ADR 0126 rule 9.) In edition `0.5`, each bound of the `range` of a type MAY also name a module constant, `bound = signed_int | lower_ident`. The name MUST resolve to a constant of an integer type (U1001 if it does not resolve, U2002 if it is not a constant or not an integer), whose value is substituted before the range is read; a constant MUST NOT be used in its own type (U1011). The ranges of expressions and patterns keep their integer bounds.

## 3. Predicates and functions

```ebnf
predicate_decl  = [ "pub" ] "predicate" ( "reflect" | "opaque" ) lower_ident
                  "(" [ params ] ")" "->" type "=" inline_or_block ;

function_decl   = [ "pub" ] signature
                  ( "=" line_expr NEWLINE                                   (* short form *)
                  | NEWLINE [ INDENT clause { clause } DEDENT ]
                    "=" NEWLINE INDENT block DEDENT ) ;                     (* long form *)
signature       = lower_ident [ type_params ] "(" [ params ] ")" "->" type ;
params          = param { "," param } ;
param           = lower_ident ":" type ;
clause          = "requires" line_expr NEWLINE
                | "ensures" line_expr NEWLINE
                | "uses" cap_ref ":" cap_type NEWLINE
                | "fails" upper_ident NEWLINE
                | "derived" "from" upper_ident { "," upper_ident } NEWLINE ;

inline_or_block = line_expr NEWLINE | NEWLINE INDENT block DEDENT ;
```

Clause order is canonical: all `requires`, then all `ensures`, then all `uses`, then `fails`, then `derived from`. At most one `fails` and one `derived from` clause (U0102).

(v0.2, ADR 0062.) In edition `0.2`, a function header also takes `"uses" "of" lower_ident NEWLINE` and `"fails" "of" lower_ident NEWLINE`. `of` is not a keyword: it is recognized only right after `uses` or `fails` in a clause. A `uses of` clause is ordered with the other `uses` clauses. A header has at most one `fails` clause of each form.

(v0.2, ADR 0066.) In edition `0.2`, the type parameters of a function may carry a constraint, as `sort[T: Ord]` (`spec/20 §2`). Those of kinds may not.

## 4. Capabilities, handlers, policies

```ebnf
capability_decl = [ "pub" ] "capability" upper_ident NEWLINE
                  INDENT op_decl { op_decl } DEDENT ;
op_decl         = "op" lower_ident "(" [ params ] ")" "->" type NEWLINE
                  [ INDENT op_clause { op_clause } DEDENT ] ;
op_clause       = "requires" line_expr NEWLINE
                | "ensures" line_expr NEWLINE
                | "fails" upper_ident NEWLINE
                | "accepts" label NEWLINE ;

handler_decl    = [ "pub" ] "handler" lower_ident "(" [ params ] ")" "for" upper_ident NEWLINE
                  INDENT { "uses" cap_ref ":" cap_type NEWLINE }
                  handler_op { handler_op } DEDENT ;
handler_op      = "op" lower_ident "(" [ lower_ident { "," lower_ident } ] ")"
                  "=" inline_or_block ;

policy_decl     = [ "pub" ] "policy" lower_ident NEWLINE INDENT
                    "from" type NEWLINE
                    "to" type NEWLINE
                    "via" lower_ident NEWLINE
                    "purpose" text_lit NEWLINE
                    "owner" lower_ident NEWLINE
                  DEDENT ;
```

Operation clause order is canonical: all `requires`, then all `ensures`, then `fails`, then `accepts`. At most one `fails` and one `accepts` clause (U0102, ADR 0017).

## 5. Requirements and examples

```ebnf
requirement_decl = "requirement" upper_ident NEWLINE INDENT
                     "text" text_lit NEWLINE
                     "source" source NEWLINE
                     "owner" lower_ident NEWLINE
                     "status" ( "proposed" | "active" ) NEWLINE
                   DEDENT ;
source          = "human" | ( "policy" | "regulation" | "generated" ) text_lit ;

example_decl    = "example" text_lit NEWLINE INDENT
                    { "given" cap_ref "=" handler_call NEWLINE }
                    "do" line_expr NEWLINE
                    expectation { expectation }
                    [ "stop" "after" integer_literal NEWLINE ]    (* edition 0.4, spec/26 §8 *)
                  DEDENT ;
test_decl       = "test" text_lit NEWLINE INDENT             (* edition 0.3, ADR 0105 *)
                    { "given" cap_ref "=" handler_call NEWLINE }
                    "do" line_expr NEWLINE
                    expectation { expectation }
                    [ "stop" "after" integer_literal NEWLINE ]    (* edition 0.4, spec/26 §8 *)
                  DEDENT ;
handler_call    = lower_ident "(" [ args ] ")" ;
expectation     = "expect" ( "fails" upper_ident
                           | [ "not" ] "performed" cap_ref "." lower_ident
                           | line_expr ) NEWLINE ;
```

After `expect`, the parser checks for `fails`, `performed` and `not performed` before falling back to `line_expr`.

(v0.3, ADR 0105.) `test_decl` is the test declaration of edition `0.3`, with the body of `example_decl`; `example_decl` is the one of editions `0.1` and `0.2` (`spec/08 §2`). In edition `0.3`, `example` at the start of a declaration is U0103 with the exact rewrite to `test` (`spec/01 §5`), and the parser reads the declaration as `test_decl`.

## 6. Blocks and statements

```ebnf
block           = { statement } final ;
statement       = let_stmt | handled ;                    (* a bare handled statement MUST have type Unit, U2011 *)
final           = handled | block_expr ;
let_stmt        = "let" lower_ident [ ":" type ] "=" ( handled | block_expr ) ;
handled         = line_expr [ with_clause ] NEWLINE [ INDENT handling { handling } DEDENT ] ;
with_clause     = "with" cap_binding { "," cap_binding } ;
cap_binding     = cap_ref "=" ( cap_ref [ "[" lower_ident { "," lower_ident } "]" ] | handler_call ) ;   (* the brackets: v0.3, ADR 0093 rule 4 *)
handling        = "if" "error" ( "fail" variant_expr | "pass" | "use" line_expr ) NEWLINE
                | "if" "none" ( "fail" variant_expr | "use" line_expr ) NEWLINE ;
variant_expr    = upper_ident [ call_args ] ;

block_expr      = when_expr | if_block ;
when_expr       = "when" line_expr NEWLINE INDENT arm { arm } DEDENT ;
arm             = pattern "->" ( line_expr NEWLINE | NEWLINE INDENT block DEDENT ) ;
pattern         = "is" upper_ident [ "(" bind { "," bind } ")" ]
                | "is" literal                             (* U4008: no interpolation, spec/07 §7 rule 3 *)
                | "in" range
                | "else" ;
bind            = lower_ident [ ":" lower_ident ] ;       (* field, or field: local name *)
if_block        = "if" line_expr NEWLINE INDENT block DEDENT
                  "else" ( NEWLINE INDENT block DEDENT | if_block ) ;
```

(v0.3, ADR 0093 rule 4.) The brackets of `cap_binding` narrow a rebinding to the listed operations, `with @x = @y[read]` (`spec/05 §4` rule 4); before edition `0.3` they are U0101. This grammar is the one that ADR 0093's Amends list names `spec/02 §4`; it lives here, in §6, with `with_clause`.

(v0.2, ADR 0065.) In edition `0.2`, a block also takes `together` statements, and a `let` the `together for` form (`spec/19 §1`).

(v0.5, ADR 0137 rules 2 and 3.) In edition `0.5`, a binding statement is written `const` or `var`:

```ebnf
let_stmt        = ( "const" | "var" ) lower_ident [ ":" type ] "=" ( handled | block_expr ) ;
```

The production keeps its name, as `spec/09` keeps the Core `let`. `const x = e` is what `let x = e` is in edition `0.4`, and `var x = e` what `let edit x = e` is (`spec/17 §1`), with the same typing, desugaring, verification and information flow. `const` binds a name to a value that does not change; at the top level of a module its type MUST be written and its value MUST be a constant expression (ADR 0126 rule 3), and in a block it MAY be any expression. `const edit x` and `var edit x` are U0101, since `edit` and `take` mark only parameters, arguments and receivers. A `var` at the top level of a module is U0101, `` a module declares no `var`; a module value is a `const` ``: a module has no mutable state. A `const` at the top level is a module constant (§1, ADR 0126). `let` is U7007 in edition `0.5` (`spec/01 §5`). Wherever this chapter and the others name a `let` of the surface language, edition `0.5` reads `const`, and a `let edit`, `var`; a Core `let` (`spec/09`) stays.

(Edition 0.4, ADR 0117 rule 5.) In edition `0.4`, a `handled` line MAY bound its expression by a deadline, after its `with` clause, and `attempt` MAY take one, which it then holds (`spec/27 §5`):

```ebnf
handled         = line_expr [ with_clause ] [ "within" add_expr ] NEWLINE [ INDENT handling { handling } DEDENT ] ;
attempt_expr    = "attempt" postfix [ "within" add_expr ] ;
```

A `handled` whose `line_expr` is an `attempt` with its own `within` takes no second `within` (U0101).

(Edition 0.4, ADR 0117 rule 6.) In edition `0.4`, a `block_expr` MAY also be a race of two or more branches, `first` followed by `of` and the end of its line opening it (`spec/27 §6`):

```ebnf
block_expr      = when_expr | if_block | together_for | first_of ;
first_of        = "first" "of" NEWLINE INDENT handled handled { handled } DEDENT ;
```

(Edition 0.4, ADR 0117 rule 7.) In edition `0.4`, a statement MAY also be a `tasks` statement, whose block starts tasks with `start` (`spec/27 §7`). `tasks` begins one where a newline and an indented block, or `at`, follow it at the start of a statement, and `start` where a name, a capability reference or `attempt` follows it; elsewhere both are names. A `with` clause after the expression of `start` parses, and is U2035.

```ebnf
statement       = ... | tasks_stmt | start_stmt ;
tasks_stmt      = "tasks" [ "at" "most" add_expr "at" "once" ] NEWLINE INDENT block DEDENT ;
start_stmt      = "start" line_expr [ "within" add_expr ] NEWLINE [ INDENT handling { handling } DEDENT ] ;
```

## 7. Expressions

```ebnf
line_expr       = if_line | lambda | checked_expr | attempt_expr | fail_expr | implies_expr ;
if_line         = "if" line_expr "then" line_expr "else" line_expr ;
lambda          = "fn" "(" [ lambda_param { "," lambda_param } ] ")" "->" line_expr ;
lambda_param    = lower_ident [ ":" type ] ;
checked_expr    = "checked" "(" line_expr "," line_expr ")" "else" "fail" variant_expr ;
attempt_expr    = "attempt" postfix ;
fail_expr       = "fail" variant_expr ;

implies_expr    = or_expr [ "implies" implies_expr ] ;
or_expr         = and_expr { "or" and_expr } ;
and_expr        = not_expr { "and" not_expr } ;
not_expr        = "not" not_expr | cmp_expr ;
cmp_expr        = add_expr [ cmp_op add_expr | "is" ( upper_ident | literal ) | "in" range ] ;
                                                         (* the literal: U4008, no interpolation, spec/07 §7 rule 3 *)
cmp_op          = "==" | "!=" | "<" | "<=" | ">" | ">=" ;
add_expr        = mul_expr { ( "+" | "-" ) mul_expr } ;
mul_expr        = unary { ( "*" | "/" | "mod" ) unary } ;
unary           = "-" unary | postfix ;
postfix         = primary { "." lower_ident [ call_args ] | call_args } ;
primary         = literal
                | lower_ident
                | upper_ident
                | cap_ref "." lower_ident call_args
                | cap_ref                                  (* accepted only to report U3005 *)
                | "(" line_expr ")"
                | "[" [ line_expr { "," line_expr } ] "]"
                | "result" | "it" | "error" ;
call_args       = "(" [ arg { "," arg } ] ")" ;
arg             = [ lower_ident ":" ] line_expr ;
literal         = int_lit | text_lit | "true" | "false" | "unit" ;
```

(v0.3, ADR 0114.) In edition `0.3`, `literal` also has `float_lit` (`spec/29 §2`), and an argument's label may be the keyword `from`, as in `x.copy_sign(from: y)` (`spec/29 §3`): `arg = [ ( lower_ident | "from" ) ":" ] line_expr`. Nowhere else does `from` serve as a name.

## 8. Context restrictions

These rules are enforced after parsing and reported with the listed codes.

| Rule | Code |
| --- | --- |
| `result` appears only in `ensures` clauses and `expect` lines | U2012 |
| `it` appears only in `where` clauses and the second argument of `checked`. (v0.3, ADR 0092.) Only the `where` of a `kind`: not in an operation's `where` (`spec/05 §3`), where the message is "`it` names no value in an operation's condition" | U2013 |
| `error` appears only inside the `fail` or `use` part of an `if error` handling clause | U2013 |
| A capability operation, or a call to a function that uses capabilities or can fail, appears only as the whole `line_expr` of a `handled` statement, `let` (in edition `0.5`, `const` or `var`) or `final`, optionally wrapped in `attempt` (one effect per line). Example `do` lines are exempt (`spec/08 §2`) | U3013 |
| A bare `cap_ref` (not followed by `.op(...)`) appears only on either side of a `with` binding or after `given @x =` | U3005 |
| `policy` appears only in `trusted` modules | U4004 |
| `given` handlers and `with` handler instances name a `handler` for the capability of the bound reference | U3010 |
| An `else` arm is the last arm of a `when` | U2008 |

A lambda's body is none of the positions of `result`, `it` and `error` above, even when the lambda is written in one: its body runs wherever the lambda is called, where they name no value. This holds for every lambda, a lambda given for an `of` parameter (`spec/05 §9`) included. `result` in a lambda's body is U2012, and `it` or `error` there is U2013; a value the body needs is passed as an argument, as in `expect (fn(x: Int) -> x + 1)(result) == 2` (#1989).

## 9. Limits

An implementation MUST accept nesting up to 256 levels, and MUST reject deeper nesting with U0101 at the token where the limit is exceeded (ADR 0019). One level is each nested line expression (parentheses, arguments, list items, interpolations, branches, lambda bodies), each prefix `not` or `-`, each operator of a chain (`a + b + c` counts two), each call, field or method in a postfix chain, each nested type, and each nested block, including each `else if`. The rest of the declaration is skipped by the usual recovery.

(#2264.) Nesting in the source is not the only depth: elaboration (`spec/09 §3`) makes each statement of a block, and each intermediate value that an expression computes (the result of a call or an operator, and each record, variant or list it builds), one `let` of Core IR whose body is the rest, so a block of many statements or a list literal of many calls or records is a Core expression as deep as it is long. The depth counts each nested Core expression. An implementation MUST accept a declaration whose Core IR nests up to 131,072 levels, and the reference implementation rejects a deeper one with U0101 at the position where the limit is exceeded, so that no pass over Core IR runs out of stack: the fix is to split the declaration into helper functions. The module constants that a declaration names count toward this depth with the records, variants and lists of their values (ADR 0126 rule 8), and the reference implementation caps them first: those that one function body, one handler operation, the `given` and `do` lines of one test together, or one `expect` line names, each counted once, and every other reference each time, may hold at most 4,096 records, variants and lists together, U2041 past it (#1632, #2264). The same parts bound the size of what substitution builds in the whole package: each constant counts what substituting it writes, its elements and fields, the characters of its texts and the decimal digits of its integers and decimal coefficients (#2317), once in each part that names it, so a constant named in k `expect` lines counts k times; a scalar value, such as a text, is copied at every reference and counts at every reference, in a part or not; and the package may substitute at most 10,000,000 of them, U2041 past it. A declaration that passes either cap is left out of the program, as one past the depth bound is, so a rejected package never builds the Core IR the caps refuse (#2316); a constant whose own value passes 10,000,000 is U2041 at its value, before it is built whole.

(ADR 0096.) After a syntax error, parsing resumes at the next declaration, and the later phases (names, types, canonical form) run on the declarations that parsed. A declaration that holds a syntax error is left out, and the names it would have declared are unknown, so no diagnostic cascades from it (`compiler/architecture.md §5`). A foreign shape, such as `if … then` at the end of a line or `=` at the end of a function header, stays a U0101: this grammar is the only accepted syntax, and the error's first repair rewrites the shape when the rewrite is exact (`spec/13 §3`).

A text literal nested in an interpolation, which is U0006, has no interpolation of its own: `{` and `}` inside it are text.
