# Architecture

This document explains how PianoMate is put together. Read it before your first change. It covers where each piece lives, how a key press becomes feedback on screen, and the rules that keep the app responsive and correct.

## The big picture

```
                ┌──────────────────────────── crates/pianomate (the app) ───────────────────────────┐
 MIDI keyboard ─┤ audio.rs: Midi ──(note, time)──┐                                                  │
 computer keys ─┤ app.slint: typed/note ─────────┼──► main.rs: Ctl ──► pianomate-core (pure logic)   │
 mouse         ─┤ keyboard.slint ────────────────┘      │   ▲         lesson / rhythm / drill /       │
                │                                       │   │         music / notation / curriculum   │
                │                     properties, models│   │callbacks                                │
                │                                       ▼   │                                         │
                │                               ui/*.slint (Slint UI)                                 │
                │                                                                                     │
                │  audio.rs: Audio ◄── rtrb ring buffer (Msg) ── any thread                           │
                │     realtime thread: rustysynth SoundFont synth + metronome ──► cpal ──► speakers   │
                └─────────────────────────────────────────────────────────────────────────────────────┘
   content/ (RON lessons, curriculum, images)  ──read at startup──►  Ctl
   progress.ron (user data dir)               ◄──atomic write────   Ctl
```

The project has two crates:

| Crate | Role | May depend on |
|---|---|---|
| `pianomate-core` | Everything that can be decided without a screen or a sound card: parsing, lesson flow, grading, drill generation, staff layout | `serde`, `ron` only |
| `pianomate` | Windows, sound, MIDI, files on disk. It turns core decisions into pixels and audio | `pianomate-core`, Slint, cpal, rustysynth, midir, rtrb, directories |

**The main rule:** if logic can be tested without a UI, it belongs in `pianomate-core` and gets a unit test. The app crate is glue.

## pianomate-core

| Module | Responsibility |
|---|---|
| `lib.rs` | `Pitch` (MIDI number, parsed from `C#4`), `note_name`, `is_black`, the QWERTY-to-note map |
| `music.rs` | The text music format (`C4/2@1`, chords, rests, `key=`), `Part`/`Music`. It merges the hands into *moments* (notes that sound together) and works out each hand's position and fingering |
| `lesson.rs` | The lesson file format (`Lesson`, `Step`, `Show`) with validation, and `Session`, the state machine that walks a learner through the steps |
| `rhythm.rs` | `RhythmRun`: grades timed playing against expected onsets (early, late, missed, extra, held long enough). `median_offset` is used for latency calibration |
| `drill.rs` | A seeded `Rng`, adaptive note-reading stats (`CardStats`), interval questions, and the generators for sight-reading, rhythm reading and chord finding. Generated lessons are ordinary `Lesson`s |
| `notation.rs` | Lays out music as a display list (`Item`/`Shape`: glyphs, rects, text, braces) in staff units, plus `Layout.time`, a list of (beat, x) points for the playhead |
| `curriculum.rs` | Books, stages, goals and emblems, with consistency checks against the lesson ids |

### The lesson session

`Session` is the heart of a lesson. It knows the current step and what the learner must do next:

- **`target()` / `targets()` / `moments()`**: the notes expected now, and for the whole passage.
- **`note_on(note)`**: feed it a key press. It returns `Some(true)` for correct, `Some(false)` for wrong, and `None` if nothing was expected.
- **`answer(choice)`**: answers a quiz step.
- **`can_advance()`, `advance()`, `back()`, `restart()`, `jump(step)`**: navigation.
- **`marks()`**: per-note colours for the staff (done, current, wrong).

`Lesson::timed_steps()` lists the steps that count toward mastery stars.

### Display lists, not widgets

Notation is laid out in core and drawn in the UI. `notation::layout` returns positioned SMuFL glyphs (Bravura font) and shapes. `staff.slint` only places them. This keeps engraving logic testable: the tests check staff positions, ledger lines, bar alignment and the playhead.

## pianomate (the app)

### Threads

| Thread | Does | Must never |
|---|---|---|
| **UI thread** (Slint event loop) | Owns `Ctl` and all app state, handles callbacks and timers | Block on I/O longer than a frame. The only exception is saving `progress.ron` |
| **Audio thread** (cpal callback) | Renders the synth and the metronome click in chunks | Allocate, lock, or block. It receives work only through the lock-free `rtrb` queue |
| **MIDI threads** (midir, one per port) | Forward raw MIDI straight to the synth for low latency, then post the key to the UI thread | Touch UI state directly |

There is one clock: `audio::now_ms()`. Key presses, metronome clicks and playheads are all measured on it, which is what makes the timing grades meaningful.

### From key press to feedback

1. **Sound.** A MIDI key sounds immediately: the midir callback pushes `Msg::Midi` to the audio queue. Computer keys and on-screen keys do the same from the UI's `note` callback.
2. **To the UI thread.** The key and its timestamp reach `key-played` on the UI thread via `upgrade_in_event_loop`.
3. **Grading.** `Ctl::played` routes the key to whatever is active:
   - a lesson `Session`
   - a timed `RhythmRun`
   - a drill
   - latency calibration
4. **Display.** `Ctl` writes the results to Slint properties and models (`show*`/`update` methods), and Slint redraws.

### Ctl: the controller

`main.rs` holds `Ctl`, a single `Rc<RefCell<…>>` shared by every Slint callback. It owns:

| Area | Fields |
|---|---|
| Lessons | the loaded lessons and curriculum, the current `Session` |
| Practice | the drill state, the `RhythmRun`, the metronome flag |
| Persistent | `Save`, written to `progress.ron` |
| Devices | the `Audio` handle and the `Midi` connections |
| Timers | a 16 ms tick, used only during timed runs; the 2 s MIDI hot-plug scan; Listen playback timers |

Key methods: `navigate`, `open`, `enter_step`, `played`, `start_rhythm` / `tick` / `finish_rhythm`, `show`, `save`.

**Generated lessons.** Lessons whose id starts with `practice-` are built by `drill.rs`. They don't count toward the road and are never saved as the resume point.

**Listen playback.** Playback is scheduled on timers tagged with a generation number. Leaving or restarting a step bumps the number, so stale notes never sound.

### UI (`crates/pianomate/ui`)

| File | Contents |
|---|---|
| `app.slint` | The root window: every property and callback the Rust side uses |
| `shell.slint` | Sidebar, Today, Road, stage page, Practice, Library, Setup, onboarding |
| `lesson.slint` | Lesson view, hand-position panel, completion screen, calibration |
| `drill.slint` | Drill screens |
| `figures.slint` | Drawn illustrations and the labelled keyboard diagram |
| `staff.slint` | Draws the notation display list |
| `keyboard.slint` | The on-screen piano |
| `widgets.slint` | Buttons, chips, cards, toggles, page scaffolding |
| `theme.slint` | `Theme` global: all colours and fonts for light and dark |

Never hard-code a colour; use `Theme`. Fonts (Bravura, EB Garamond) are embedded at compile time.

### Persistence

`Save` (in `main.rs`) is serialised as RON to `progress.ron` in the platform data directory. Writes go to a temporary file followed by a rename, so a crash can't corrupt progress.

New fields must use `#[serde(default)]`, so older save files keep loading. A derived `Default` ignores `serde(default = …)` functions, so choose field meanings where zero is a sensible value.

### Content loading

At startup the app reads `content/curriculum.ron`, every `content/lessons/*.ron`, and images on demand. Paths are relative to the working directory. A packaged release falls back to the folder that holds the executable. A broken lesson file stops startup with an error that names the file.

## Content

```
content/
  curriculum.ron     books[], stages[] (id, part, emblem, image, lessons[]), goals[] (stage order per goal)
  lessons/*.ron      one Lesson per file; file name = id
  images/            public-domain art; every file listed in CREDITS.md
```

Shipped content is tested:

| Test | Checks |
|---|---|
| `every_shipped_lesson_parses` | Every lesson loads and validates |
| `every_shipped_passage_fills_whole_bars` | No partial bars, and every visible Play step has tips |
| `shipped_curriculum_is_consistent` | Ids are unique, references resolve, and every lesson is placed |

## Release

`.github/workflows/release.yml` runs when a `v*` tag is pushed. For Linux (Ubuntu 22.04) and Windows it:

1. tests and builds in release mode;
2. bundles the executable with `content/`, a GeneralUser GS SoundFont pinned to a commit, the SoundFont's licence and the project licence;
3. attaches the archive to a GitHub release.

## Known shortcuts

Limits are marked in the code with `// shortcut: <limit>, <when to upgrade>`. Run `grep -rn "shortcut:" crates` to list them. Current ones:

| Limit | Where |
|---|---|
| Notation draws flags, never beams; every note gets its own accidental; one line, no system breaks | `notation.rs` |
| Audio output requires an f32 device | `audio.rs` |
| The computer-key mapping assumes a QWERTY layout | `lib.rs` |
| Streak days roll over at midnight UTC | `main.rs` |
| Content is read from disk, not embedded | `main.rs` |
