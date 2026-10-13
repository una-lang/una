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
                | handler_decl | policy_decl | requirement_decl | example_decl ;
```

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
cap_op          = lower_ident [ "where" line_expr ] ;
```

A `cap_op` ends at the first `,` or `]` at bracket and parenthesis depth 0: every nested `,`, `[ ]` or `( )` belongs to the expression. A function type's `uses` (`fn_type_clause`) keep `cap_type`, so `where` there is U0101 in every edition, as it is in a header's `uses` in editions `0.1` and `0.2`. The condition's meaning is in `spec/05 §3`.

After a `level`, `[` always starts the label's categories. A labeled list type is written with the named form, `secret List[Int]`, or after the categories, `secret[PII] [Int]`. A label with categories that is not followed by a type is U0101 (ADR 0018).

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
                  DEDENT ;
handler_call    = lower_ident "(" [ args ] ")" ;
expectation     = "expect" ( "fails" upper_ident
                           | [ "not" ] "performed" cap_ref "." lower_ident
                           | line_expr ) NEWLINE ;
```

After `expect`, the parser checks for `fails`, `performed` and `not performed` before falling back to `line_expr`.

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
                | "is" literal
                | "in" range
                | "else" ;
bind            = lower_ident [ ":" lower_ident ] ;       (* field, or field: local name *)
if_block        = "if" line_expr NEWLINE INDENT block DEDENT
                  "else" ( NEWLINE INDENT block DEDENT | if_block ) ;
```

(v0.2, ADR 0065.) In edition `0.2`, a block also takes `together` statements, and a `let` the `together for` form (`spec/19 §1`).

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

## 8. Context restrictions

These rules are enforced after parsing and reported with the listed codes.

| Rule | Code |
| --- | --- |
| `result` appears only in `ensures` clauses and `expect` lines | U2012 |
| `it` appears only in `where` clauses and the second argument of `checked`. (v0.3, ADR 0092.) Only the `where` of a `kind`: not in an operation's `where` (`spec/05 §3`), where the message is "`it` names no value in an operation's condition" | U2013 |
| `error` appears only inside the `fail` or `use` part of an `if error` handling clause | U2013 |
| A capability operation, or a call to a function that uses capabilities or can fail, appears only as the whole `line_expr` of a `handled` statement, `let` or `final`, optionally wrapped in `attempt` (one effect per line). Example `do` lines are exempt (`spec/08 §2`) | U3013 |
| A bare `cap_ref` (not followed by `.op(...)`) appears only on either side of a `with` binding or after `given @x =` | U3005 |
| `policy` appears only in `trusted` modules | U4004 |
| `given` handlers and `with` handler instances name a `handler` for the capability of the bound reference | U3010 |
| An `else` arm is the last arm of a `when` | U2008 |

## 9. Limits

An implementation MUST accept nesting up to 256 levels, and MUST reject deeper nesting with U0101 at the token where the limit is exceeded (ADR 0019). One level is each nested line expression (parentheses, arguments, list items, interpolations, branches, lambda bodies), each prefix `not` or `-`, each operator of a chain (`a + b + c` counts two), each call, field or method in a postfix chain, each nested type, and each nested block, including each `else if`. The rest of the declaration is skipped by the usual recovery.

(ADR 0096.) After a syntax error, parsing resumes at the next declaration, and the later phases (names, types, canonical form) run on the declarations that parsed. A declaration that holds a syntax error is left out, and the names it would have declared are unknown, so no diagnostic cascades from it (`compiler/architecture.md §5`). A foreign shape, such as `if … then` at the end of a line or `=` at the end of a function header, stays a U0101: this grammar is the only accepted syntax, and the error's first repair rewrites the shape when the rewrite is exact (`spec/13 §3`).

A text literal nested in an interpolation, which is U0006, has no interpolation of its own: `{` and `}` inside it are text.
