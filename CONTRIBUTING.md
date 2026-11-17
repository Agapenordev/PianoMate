# Contributing to PianoMate

Thank you for helping. There are three ways to contribute, and you don't need to be a programmer for the first one:

1. **Lessons and content.** Write or improve lessons in plain text files. This needs piano knowledge, not Rust.
2. **Code.** Features, fixes and notation. See [ROADMAP.md](ROADMAP.md) and [ARCHITECTURE.md](ARCHITECTURE.md).
3. **Testing and feedback.** Play through lessons with a real keyboard and report what was confusing, wrong or slow.

AI coding agents: read [AGENTS.md](AGENTS.md) as well.

## Setup

```sh
git clone https://github.com/Agapenordev/PianoMate && cd PianoMate
sudo apt install libasound2-dev      # Linux; openSUSE: sudo zypper install alsa-devel
mkdir -p assets && curl -L -o assets/piano.sf2 \
  https://github.com/mrbumpy409/GeneralUser-GS/raw/main/GeneralUser-GS.sf2
cargo run          # the app
cargo test         # must pass before every pull request
```

Dev builds compile dependencies with optimisation, so the synth doesn't crackle. The first build takes a few minutes.

## Writing a lesson

1. **Create the file.** Copy a lesson close to what you want, such as `content/lessons/f17-a-minor.ron`, into a new file named after its `id`.
2. **Add it to a stage.** Add the id to a stage's `lessons` list in `content/curriculum.ron`. If you create a new stage, also add it to each goal's `stages` list where it belongs.
3. **Check it.** Run `cargo test`. It checks that the lesson parses, that every bar is full, and that every Play step has `tips`.
4. **Play it.** Play through it in the app with a keyboard. Is every instruction clear without a teacher in the room?

The music syntax and step types are summarised in the [README](README.md#writing-lessons), and documented fully in `crates/pianomate-core/src/lesson.rs`.

### What makes a good lesson

- **One idea per lesson.** Show it, make the learner touch it, then use it in music.
- **Visual first.** Prefer a keyboard diagram, a figure or a staff over a paragraph. Keep explanations to two or three sentences.
- **Every passage teaches something.** Give it a clear prompt and two concrete tips: what the hand does, and what to listen for.
- **Correct fingering on every note** where it isn't obvious. Look at the hand-position panel; if it looks wrong, the fingering probably is.
- **Short passages.** Keep them to about four bars until notation supports system breaks.
- **Original or public domain only.** Write in your own words. Never copy text, exercises or arrangements from copyrighted method books. Melodies must be public domain, such as folk tunes or classical works whose composers died long ago, or your own.
- **Images.** Images must be public domain or openly licensed. Add each one to `content/images/CREDITS.md` with its source and licence.

## Code contributions

- **Find something to work on.** Pick an item from [ROADMAP.md](ROADMAP.md) or an issue. Comment that you're on it.
- **Before you start:** read [ARCHITECTURE.md](ARCHITECTURE.md). Pure logic goes in `pianomate-core` with a test; the app crate is glue.
- **Keep changes small and focused.** One feature or fix per pull request. Smaller diffs get reviewed faster.
- **Follow the surrounding code.** Use its naming, `Theme` colours (never hard-coded), and existing widgets in `widgets.slint`.
- **Comments.** Write a comment only for the *why* the code can't show. Mark known limits with `// shortcut: <limit>, <when to upgrade>`.
- **Save compatibility.** Never break existing save files. New `Save` fields need `#[serde(default)]`.
- **Real-time safety.** Nothing on the audio thread may allocate, lock or block.
- **UI changes.** Include a before/after screenshot in the pull request. Check both light and dark themes.

### Pull request checklist

- [ ] `cargo test` passes
- [ ] `cargo build` has no warnings
- [ ] New logic in `pianomate-core` has a test
- [ ] UI changes are screenshotted in light and dark themes
- [ ] README, ARCHITECTURE or ROADMAP updated if behaviour or structure changed

## Reporting bugs

Include:
- your OS;
- your MIDI keyboard model, if any;
- your audio device;
- what you did, what you expected, and what happened;
- the lesson and step, if the bug is in a lesson.

Run the app from a terminal to see error output: `./pianomate` on Linux, or `cargo run` from source.

## Licence

By contributing, you agree that your contribution is licensed under the [MIT License](LICENSE).
