# Roadmap

**The goal:** take someone from their first touch of the keys to playing demanding arrangements, such as virtuoso anime and film piano covers. It should feel like a patient teacher who listens to every note.

Status as of v0.1.0. Items are roughly in priority order within each section. Want to take one? Open an issue saying so, so work isn't duplicated. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Done (v0.1.0)

- **Road.** 27 stages and 47 lessons across Foundations, Technique, Repertoire and Arranging, with four goal-based roads.
- **Lessons.**
  - Explanations with figures and keyboard diagrams, quizzes, find-the-keys tasks.
  - Music for one or both hands on a grand staff, rhythm taps and by-ear steps.
  - A hand-position panel and coaching tips on every passage.
- **Feedback.**
  - Note-by-note grading.
  - In-time mode with a count-in and playhead, held-note checks, and tempo at 60, 80 or 100%.
  - Latency calibration.
- **Practice.** Adaptive note reading, interval ear training, generated sight-reading, rhythm reading and chord finding, and a metronome.
- **Progress.** Mastery stars, resuming at the exact step, a daily plan, streaks and weekly minutes.
- **Platform.**
  - Light and dark themes.
  - MIDI hot-plug, computer-key input, an optional on-screen piano.
  - Windows and Linux release packages.

## Next: v0.2 (finishing the Foundations)

- **More minor keys.** D and E minor, the harmonic minor scale, and minor primary chords (i, iv, V7).
- **F major position and the B♭ key signature.** The F major sight-reading drill already exists; the road lessons don't yet.
- **Pedal.** Read CC64 from MIDI and teach the sustain pedal: legato pedalling, then pedal marks in notation.
- **Dynamics graded from velocity.** Use the key velocity a MIDI keyboard already sends to check *p*, *f*, crescendos and a melody voiced over its accompaniment.
- **Notation quality.**
  - Beamed eighths.
  - Accidentals that carry through the bar.
  - System breaks, so pieces longer than about four bars fit.
- **Review that remembers.** Spaced review of finished lessons, suggested by Today when a skill is going stale.

## Later: v0.3 (technique and repertoire)

- **Scale and arpeggio system.** All major and minor keys, hands together, contrary motion, with tempo ladders tracked over weeks.
- **Longer repertoire.** Complete short pieces (Burgmüller, Clementi sonatinas, easier Chopin) with section-by-section practice:
  - loop a bar range;
  - hands-separate mode;
  - slow-practice tempo ramps.
- **Wait mode with accompaniment.** The app plays the other hand, or a backing, and waits for you.
- **Practice statistics.** Accuracy and tempo history per passage. Show where you stumble and turn that into exercises.
- **macOS build** in the release workflow.

## Toward the goal: v0.4+ (arranging and advanced playing)

- **Arranging course.**
  - Voicings, extended chords, and left-hand textures from stride to wide arpeggios.
  - Reharmonisation.
  - Turning a melody and chord symbols into a full arrangement.
- **Ear course.** Transcribing melodies, then bass lines, then chords, using pitch detection from the MIDI input.
- **Import music.** Load MusicXML or MIDI files into the practice tools: grading, looping, tempo control.
- **Advanced technique.** Leaps, octave tremolos, fast arpeggios across the keyboard, rotation, and hand-crossing études.

## Ideas (not planned yet)

- Microphone input for acoustic pianos, using pitch detection.
- Lesson editor with a live preview.
- Translations of the UI and lessons.
- Accessibility pass: screen-reader labels and a high-contrast theme.
- Embedding the content in the binary, for a single-file build.

## Not planned

- **Content copied from copyrighted method books.** Lessons must be original writing. Musical examples must be original or in the public domain.
- **Cloud accounts, ads, or tracking.** Progress stays on the learner's machine.
