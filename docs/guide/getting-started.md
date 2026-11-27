# Getting started with Una

In about ten minutes you install Una, write a function with a contract, watch `una check` find the input that breaks it, fix it, and build a native executable.

You write `wordfreq`, a program that counts the words of a text and prints the counts as JSON. Every code block here is tested against this release, except that the outputs shown in `text` blocks are not compared with a run, and the finished program is `examples/wordfreq`.

> [!NOTE]
> Una runs on macOS and Linux on arm64. [More about installing](#more-about-installing) lists the supported systems.

## 1. Install

Run the installer:

```sh
curl -fsSL https://raw.githubusercontent.com/una-lang/una/main/install.sh | sh
```

It puts the release in `~/.una` and changes nothing else. The installer is the supported way to install. If you downloaded the macOS archive in a browser instead, read [An archive downloaded by hand on macOS](#an-archive-downloaded-by-hand-on-macos). Put `~/.una/bin` on your `PATH`:

```sh
export PATH="$HOME/.una/bin:$PATH"
```

To keep it for new shells, add the same line to your shell's profile: `~/.zshrc` for zsh, the default shell on macOS, or `~/.bashrc` for bash.

Check that it works:

```sh
una --version
```

It prints the release, the editions it serves and the version of the prover built into it. That is everything you need to check, test and run programs.

> [!TIP]
> Working with an AI agent? Give it [Una, compact](una-compact.md): the language in one page, sized for its context.

## 2. A contract, and a proof that fails

A contract says what a function needs and what it promises. Una proves it for every input, before the program runs.

Make a directory `wordfreq` with two files. `una.toml` names the package and its edition, and the module `wordfreq` lives in `src/wordfreq.una`. Start with a function that takes the first letter of a word:

`una.toml`
```toml
[package]
name = "wordfreq"
version = "0.1.0"
edition = "0.2"
```

`src/wordfreq.una`
```una
-- fails with U5001
module wordfreq

--- The first letter of a word.
pub initial(word: Text) -> Text
  ensures result.len == 1
=
  word.slice(start: 0, end: 1)
```

The line `ensures result.len == 1` is the promise: the result is one character long. In the `wordfreq` directory, run `una check .`:

```text
[U5001] Error: precondition `0 <= 0 and 0 <= 1 and 1 <= word.len` of this call is refuted
    ╭─[ src/wordfreq.una:7:3 ]
    │
  7 │   word.slice(start: 0, end: 1)
    │   ──────────────┬─────────────
    │                 ╰─────────────── precondition `0 <= 0 and 0 <= 1 and 1 <= word.len` of this call is refuted
    │
    │ Note 2: counterexample: word = Text#0, word.len = 0
```

The promise does not hold, and Una shows why:

- **The counterexample** is `word.len = 0`: an empty word.
- **The cause** is the call of `slice`, which requires its positions to lie in the text.
- **The repairs** come with the diagnostic. With `una --json check`, it is a JSON document an agent can act on.

The fix is to say what `initial` needs, with `requires`:

`una.toml`
```toml
[package]
name = "wordfreq"
version = "0.1.0"
edition = "0.2"
```

`src/wordfreq.una`
```una
module wordfreq

--- The first letter of a word.
pub initial(word: Text) -> Text
  requires word.len >= 1
  ensures result.len == 1
=
  word.slice(start: 0, end: 1)

example "a word's first letter"
  do initial("una")
  expect result == "u"
```

Now `una check .` prints nothing: every check passes.

- **Every caller has to prove the `requires`**, and the body may assume it.
- **The `example` is a test.** `una test .` runs it, and `una check` proves that `"una"` meets the `requires`.

> [!TIP]
> `una explain U5001` explains a diagnostic code, with a failing example and its fix.

## 3. A program that runs

Turn the package into a program. Three files:

- **`una.toml`** gains a `[run]` section: where `una run` starts, and what the program may use.
- **`verification.lock`** fixes the prover's version and settings, so a check that passes on your machine passes on every other. Copy it as it is.
- **`src/wordfreq.una`** gets the words of a line, and a `main` that prints how many there are.

`una.toml`
```toml
[package]
name = "wordfreq"
version = "0.1.0"
edition = "0.2"

[run]
entry = "wordfreq"
grants = ["console"]
```

`verification.lock`
```toml
encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
```

`src/wordfreq.una`
```una
module wordfreq

--- The words of a line: the parts between spaces, without the empty ones.
pub words(line: Text) -> List[Text]
=
  let space = " "
  line.split(on: space).filter(keep: fn(part) -> part.len > 0)

pub main() -> Unit
  uses @console: Console[print]
=
  let text = "the cat and the hat and the bat"
  @console.print("{words(text).len} words")
```

Three things to notice:

- **A function is an equation:** its header, then `=`, then its body.
- **`split` and `filter` are prelude functions**, called with method syntax.
- **`main` says what it uses:** `@console`, with the operation `print`. A function that calls another performs its effects too, so the caller's `uses` must cover them. `una contract` prints a function's header, its `uses` included.

Check it and run it:

```sh
una check .   # copy examples/wordfreq
una run .
```

```text
8 words
```

`una check` runs every check, the proofs included, and prints nothing when all pass. The source must also be in canonical form; `una fmt .` rewrites it so.

## 4. Maps and JSON

Count the words with a `Map`, which keeps its keys in order, and write the counts as JSON. This is the whole program, with `initial` from section 2 back in:

`una.toml`
```toml
[package]
name = "wordfreq"
version = "0.1.0"
edition = "0.2"

[run]
entry = "wordfreq"
grants = ["console"]
```

`src/wordfreq.una`
```una
module wordfreq

--- The words of a line: the parts between spaces, without the empty ones.
pub words(line: Text) -> List[Text]
=
  let space = " "
  line.split(on: space).filter(keep: fn(part) -> part.len > 0)

--- The first letter of a word.
pub initial(word: Text) -> Text
  requires word.len >= 1
  ensures result.len == 1
=
  word.slice(start: 0, end: 1)

--- How often each word occurs, in ascending order of the words.
pub counts(words: List[Text]) -> Map[Text, Int]
=
  let edit counts: Map[Text, Int] = empty_map()
  for word in words
    let seen = counts.get(key: word)
      if none use 0
    edit counts.set(key: word, value: seen + 1)
  counts

--- The counts as one JSON object.
pub report(counts: Map[Text, Int]) -> Text
=
  let edit fields: Map[Text, Json] = empty_map()
  for entry in counts.entries()
    edit fields.set(key: entry.key, value: Number(entry.value))
  Object(fields).to_json()

pub main() -> Unit
  uses @console: Console[print]
=
  let text = "the cat and the hat and the bat"
  @console.print(report(counts(words(text))))

example "a word's first letter"
  do initial("una")
  expect result == "u"

example "each word is counted"
  do counts(words("a b a"))
  expect result == empty_map().with(key: "a", value: 2).with(key: "b", value: 1)
```

- **Values, not references.** `let edit` declares a binding that the code may change. `edit counts.set(...)` gives `counts` a new map, which is updated in place when nothing else holds it.
- **Absence is a value.** `get` returns an `Option`: `if none use 0` handles a word not seen yet.
- **One effect per line.** Each line that can fail or act is handled on the lines below it, as `if none` here.

```sh
una test .   # copy examples/wordfreq
una run .
```

```text
{"and":2,"bat":1,"cat":1,"hat":1,"the":3}
```

## 5. Capabilities and grants

`main` uses `Console[print]` and nothing else, and `una run` gives it exactly what `[run] grants` lists in `una.toml`.

- **Without `"console"` there, the program does not start:** U3012 names the capability that is missing.
- **A grant can also come from the command line:** `una run . --grant console`.

The host offers these capabilities, only to `main`, which passes them to the functions it calls:

| Capability | Since edition |
| --- | --- |
| `Console`, `Clock`, `Env`, `Files`, `Keys` | `0.1` |
| `Http` | `0.2` |
| `HttpServer`, for a program that serves HTTP | `0.4` |

Edition `0.4` offers them to the entry functions of a driven or served module too ([Serving HTTP](serving-http.md), [Driven programs](driven-programs.md)).

> [!NOTE]
> Information flow is checked too. Text that comes from the outside, as `Files.read` returns, is labeled `internal`, and printing it needs a declassification policy (§9 of [the guide for agents](una-for-agents.md)).

## 6. A native executable

`una build` checks the package, then compiles `main` into a native executable, with the grants of `una.toml` built in:

```sh
una build .   # copy examples/wordfreq
./target/una/wordfreq
```

```text
built target/una/wordfreq
{"and":2,"bat":1,"cat":1,"hat":1,"the":3}
```

The executable prints what `una run` prints, and traces the same events: the interpreter is the reference, and the test suite runs every example program in both.

> [!NOTE]
> On Linux, `una build` links the executable through `cc`, against the system's C library: without `cc` it stops with "cannot run the system linker `cc`". The executable runs on the machine that built it and on systems with the same or a newer glibc, not on musl systems.

## 7. An editor

`una lsp` is a language server (LSP 3.17 over stdin and stdout) for any editor that speaks the protocol: it checks as you type, proves a file when it is opened or saved, formats on request, and never runs programs.

## 8. Edition 0.4

`wordfreq` is a package of edition `0.2`, a stable edition. Edition `0.4` is a preview: a later release may still change what it accepts. It adds:

- programs that run until they are stopped;
- tasks and time limits;
- HTTP servers;
- dependencies on other packages through Itá, the package manager (in this release, path dependencies and registry packages read from a mirror directory).

A package of edition `0.4` is described by `ita.una` and pinned by `ita.lock`, not by `una.toml` and `verification.lock`: `una.toml` naming `edition = "0.4"` is U7002, and `una migrate` is the repair.

To move a copy of `wordfreq` to edition `0.4`:

1. Its tests are written `example`, the form of editions `0.1` and `0.2`. From edition `0.3` on they are written `test`, and `una fix` rewrites them in a package of edition `0.3` only. So first set `edition = "0.3"` in `una.toml`, then run `una fix .`, which rewrites both tests.
2. Run `una migrate .`. It writes `ita.una` and `ita.lock`, removes `una.toml` and `verification.lock`, and prints every diagnostic that the change of edition causes, none here.
3. `una check .`, `una test .` and `una run .` then work as before, in edition `0.4`.

The `ita.una` it writes:

```text
ita wordfreq
  version 0.1.0
  edition 0.4

run
  entry wordfreq
  grants
    console
```

The pages on edition `0.4` are [Tasks and time](https://github.com/una-lang/una/blob/main/docs/guide/tasks-and-time.md), [Serving HTTP](https://github.com/una-lang/una/blob/main/docs/guide/serving-http.md), [Driven programs](https://github.com/una-lang/una/blob/main/docs/guide/driven-programs.md) and [the package manager](https://github.com/una-lang/una/blob/main/docs/guide/configure/ita/commands.md); [Known limits](https://github.com/una-lang/una/blob/main/docs/guide/known-limits.md) says what this release does not do yet.

## Where next

- **With an agent:** [Una, compact](una-compact.md) is the language in one page, and [the guide for agents](una-for-agents.md) is the full reference to edition `0.2`, with notes where editions `0.3` and `0.4` differ.
- **A construct:** [the syntax reference](syntax-reference.md) lists the constructs of edition `0.2`, and some of what editions `0.3` and `0.4` add.
- **A diagnostic:** `una explain <code>` explains any code, with examples.
- **The definition:** [the specification](https://github.com/una-lang/una/tree/main/docs/spec) is normative.

## More about installing

### Supported systems

| System | Supported | What `una build` needs |
| --- | --- | --- |
| macOS on arm64 | Yes | Nothing: it links with the linker built into `una` |
| Linux on arm64, glibc 2.28 or later, from 0.5.2 (tested on Ubuntu 22.04, Debian 12 and AlmaLinux 8; Ubuntu 20.04, Debian 10 and 11 and RHEL 8 and later by the glibc symbol versions `una` needs) | Yes | A C compiler available as `cc`, such as gcc or clang with the C library's development files |
| Linux on arm64 with 0.4.0 to 0.5.1 | Yes, with glibc 2.39 or later (Ubuntu 24.04, for example) | The same |
| Linux with an older glibc (before 2.28) | No | |
| Linux with musl, such as Alpine | No | |
| Linux on x86-64 | No | |
| Windows | No | |

`una check`, `una test` and `una run` need no other program. On Linux, `una` is a dynamic executable against the system's glibc, with the C++ runtime of the solver linked into it. The installer refuses an unsupported Linux before any download, a 32-bit userland on a 64-bit kernel included.

### What the installer does

- It installs only an archive whose digest the release's validation record lists, or that `docs/validation/repacked.md` pairs with that listed digest (an archive packed again after its release).
- It changes nothing outside `~/.una`.
- To update, run it again: it replaces the release in `~/.una` with the current one, and prints the new version when it is done. It removes the files of the earlier installation that the new one does not hold, such as the `z3` and `lib/libuna_rt.a` of a release before 0.4.0, and names them.
- To uninstall, run `curl -fsSL https://raw.githubusercontent.com/una-lang/una/main/install.sh | sh -s -- --uninstall`. It removes the files the installer recorded and the key of the proof cache that `una` makes at run time, `~/.una/proof-cache.key`, and then `~/.una` when nothing else is in it. It removes no other file: what is left is named in its message, and a key file that `UNA_PROOF_KEY` names is kept.
- The README says more about the installer and the release archives.

### An archive downloaded by hand on macOS

The installer is the supported way to install. The `una` in the macOS archive of the releases page is ad hoc signed and is not notarized. A browser marks what it downloads with the `com.apple.quarantine` attribute, and the files unpacked from the archive carry it. macOS does not run that `una`: started from a terminal, the process is killed (exit code 137) and `una` prints nothing. Either use the installer, whose download carries no such attribute, or remove the attribute from the unpacked file with `xattr -d com.apple.quarantine <file>`, where `<file>` is the unpacked `una`. Compare the archive's SHA-256 digest with the one in the release's validation record first, as the installer does.

### Run `una` as an ordinary user

The solver refuses to run as root, so `una check` as root fails with U5003. Run `una` as an ordinary user, in a container too (`docker run --user 1000:1000 ...`).

### Installing as root

As root, the installer installs only into a `UNA_HOME` that only root can change:

- `UNA_HOME`, the directories it writes in and the files there are not symbolic links, and they are owned by root and writable by no one else.
- So is every directory above them, except that a directory above an existing, root-owned one may be writable by others when it is sticky, as `/tmp` is. So `UNA_HOME=/tmp/una` is refused unless `/tmp/una` already exists and is root's.
- The same holds for `TMPDIR` (default `/tmp`), where the archive is checked and unpacked, which may itself be sticky. Uninstall makes no work directory and does not check it.
- Each file is written as a new one: an existing name is removed first, so the installer never writes into a file that is a hard link to another one.
- For every user, not only root, what is at a name the installer writes must be something it may replace: where a file goes, a regular file, or for an ordinary user a symbolic link that names no directory; where a directory goes, a directory, which as an ordinary user you must be able to write in. A regular file or a link there is replaced, whoever made it. A directory or a special file is never removed to make room: it is refused, and so is a directory you cannot write in, before anything is written, so the installation does not stop halfway on either.
- Otherwise it refuses before any download, and as root it never runs the installed `una`.
- Only owners and mode bits are read: write access granted by an ACL (on macOS, a `+` after the mode in `ls -l`) is not seen.

### What `una --version` prints

It prints `una` and the release, the editions it serves, such as `editions: 0.1, 0.2, 0.3, 0.4, 0.5`, the editions among them that are previews, such as `preview: 0.3, 0.4, 0.5`, and the version of Z3, the solver built into it. A release with no preview edition prints no `preview:` line.

### If `--version` is an unexpected argument

A release older than 0.3 has no `--version` flag, and answers with an error:

```text
error: unexpected argument '--version' found
```

If you see that error after installing, the shell found an older `una` first. `which -a una` lists every `una` on your `PATH`, in the order the shell tries them. Remove the old one, or put `$HOME/.una/bin` before its directory.
