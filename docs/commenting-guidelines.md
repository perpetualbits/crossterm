# Commenting & Code-Readability Guidelines

> Standard for the whole project. **Every Claude Code prompt references this
> file.** Goal: code that is readable for both humans and AI — comments carry
> intent, invariants, and non-obvious mechanics, and are kept rigorously in sync
> with the code.

## Principles

1. Comments explain **why** and the **non-obvious what** — not a restatement of
   the code. `i += 1; // increment i` is noise; `i += 1; // skip the continuation
   cell of the wide grapheme` is signal.
2. Names do the obvious explaining; comments cover rationale, edge cases,
   invariants, units, and algorithm steps.
3. **Comments must always match the code.** A wrong comment is worse than none —
   it misleads. The sync discipline below is mandatory, not optional.

## Function comment blocks — every function gets one

Every function (public *and* private) carries a doc comment (`///`).

- **Non-trivial functions:** a full block — a one-line summary, then a prose
  explanation of the approach/algorithm (walk multi-step logic in order), plus the
  relevant rustdoc sections below.
- **Trivial functions** (plain getters, one-line forwarders): a single concise
  summary line is enough. Do **not** pad them into multi-section blocks — that is
  noise, and the standard explicitly permits brevity here.

Rustdoc sections to use when they apply (omit a section that is genuinely N/A):

```rust
/// <one-line summary of what it does>.
///
/// <detailed explanation of how it works / the algorithm. For multi-step logic,
/// describe the steps in the order they execute and call out anything subtle>.
///
/// # Parameters
/// - `name`: <meaning, units, valid range — only when not obvious from the type/name>
/// # Returns
/// <meaning of the return value>
/// # Invariants
/// <preconditions assumed, postconditions guaranteed/maintained>
/// # Panics
/// <conditions under which it panics — omit if it never does>
/// # Errors
/// <what an `Err` means — for fallible functions>
/// # Examples
/// <a compiling doctest when it genuinely aids understanding>
```

Document struct/enum **fields** too (`///` on each field) when the meaning,
units, or constraints aren't obvious from the name.

## Inline comments — on every non-trivial line or step

Add an inline `//` comment wherever the purpose, the reason, or an edge case
isn't obvious from the code alone. Treat a line as **non-trivial** when any of
these is true:

- it relies on a subtle invariant, ordering, or off-by-one/boundary handling;
- it does bit-twiddling, escape sequences, Unicode width/grapheme handling, or
  arithmetic whose intent isn't self-evident;
- it clamps, rounds, or special-cases something **and the reason matters**;
- a reader would reasonably ask "why is this here / why this way?".

Do **not** comment trivial lines (`let mut out = Vec::new();`). Inline comments
state the *why* or the non-obvious *what*, never the literal operation.

## The sync discipline (before and after every change) — MANDATORY

On every code change, perform a two-pass consistency check:

1. **Before editing:** read the target function(s) end to end and confirm the
   existing code and its comments already agree. If they don't, fix the comment
   first (call it out) so you start from a consistent state.
2. **During:** change code and its comments **together**, in the same edit — never
   leave a comment describing the old behavior.
3. **After editing:** re-read each changed function in full and verify code and
   comments still describe the same behavior — summary, parameters, return,
   invariants, panics, and **every inline comment**. No stale comment may remain.
4. If a comment cannot be made to match the code, one of them is wrong — resolve
   it before considering the change done.

State in the change/PR notes that the before/after passes were performed.

## Rust specifics

- `//!` for module-level docs, `///` for items, `//` for inline rationale.
- Prefer self-documenting names so comments can focus on *why*.
- Doc examples should compile (they run as doctests) where practical.

## Anti-patterns to avoid

- Restating the code (`return x; // return x`).
- Comments that rot: referencing line numbers, "see above", or transient state.
- Decorative banner comments with no information content.
- Stale TODOs and commented-out code left in place.
