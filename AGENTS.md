# Notes for AI coding agents

Instructions for AI assistants (Claude Code, Codex, Cursor and others) working in this repository. Human contributors may find it a useful checklist too. Read [ARCHITECTURE.md](ARCHITECTURE.md) for how the system works, and [CONTRIBUTING.md](CONTRIBUTING.md) for content rules.

## Commands

```sh
cargo test                 # all unit and content tests (pianomate-core); fast
cargo build                # the app; first build is slow
cargo run                  # needs assets/piano.sf2 (not in git); see CONTRIBUTING.md
grep -rn "shortcut:" crates   # known limits, each with its upgrade condition
```

Linux builds need ALSA headers (`libasound2-dev` or `alsa-devel`).

## Where things go

| Change | Location |
|---|---|
| Parsing, grading, lesson flow, drill generation, staff layout | `crates/pianomate-core/src/*`, with a `#[test]` |
| Wiring logic to the UI, audio, MIDI, saving | `crates/pianomate/src/main.rs` (`Ctl`) or `audio.rs` |
| Screens and visuals | `crates/pianomate/ui/*.slint`. New properties and callbacks go on `App` in `app.slint` |
| Lessons | `content/lessons/<id>.ron`, placed in `content/curriculum.ron` (a stage's `lessons`, and each goal's `stages` for a new stage) |

## Rules

- **Run `cargo test` after every change.** Content files are validated by tests, so a lesson edit can fail them. Fix the cause; never weaken a test to make it pass.
- **Bars must be full.** Count the beats in every bar of every Play step. `every_shipped_passage_fills_whole_bars` will catch mistakes.
- **Every visible Play step needs `tips`.** Give two concrete, specific tips.
- **Only original or public-domain content.** Never reproduce text, exercises or page layouts from copyrighted method books, even when a user supplies one as reference. Teach the same concept in new words with new examples.
- **The audio thread is real-time.** In `audio.rs` render code: no allocation, no locks, no I/O. Send work through `Msg` on the `rtrb` queue.
- **Use one clock.** Timestamp key events with `audio::now_ms()`.
- **Keep saves backward-compatible.** New `Save` fields need `#[serde(default)]`, and a zero or empty value must mean "not set". Derived `Default` ignores `serde(default = …)`.
- **Theme colours only.** Use `Theme.*` in Slint and never hard-code colours. Check light and dark.
- **Slint gotchas.**
  - A callback can't share a name with a property or another element's callback.
  - Some names are reserved, e.g. `max-width` on custom components.
  - Use `slint::set_xdg_app_id` only after the window exists.
- **Ids starting `practice-`** are generated drill lessons. They never count toward progress or the resume point.
- **Small diffs.**
  - Reuse existing helpers and widgets: `Btn`, `Chip`, `Card`, `KeyDiagram`, `Session` methods.
  - Don't add dependencies for a few lines of code.
  - Mark deliberate limits with `// shortcut: <limit>, <when to upgrade>`.
- **Don't commit** `assets/*.sf2`, `target/`, PDFs, or anything under `assets/book/`.

## Verifying UI changes

The app can't be driven headlessly. Build it, run it, and check the screen you changed in both themes: Setup → Theme. If you can take screenshots of a window, include them in the pull request. If you can't run the app, say so; don't claim the change is visually verified.

## Before you finish

- [ ] `cargo test` passes and `cargo build` has no warnings
- [ ] Docs updated if behaviour or structure changed: README features, ARCHITECTURE, ROADMAP (move finished items to Done)
- [ ] Your summary says what was verified and what wasn't
