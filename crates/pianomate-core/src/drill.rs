//! Practice drills: note reading that adapts to your weak notes, and interval ear training.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{Pitch, lesson::Clef};

/// Small deterministic random numbers (xorshift); drills need variety, not cryptography.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    pub fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }

    fn unit(&mut self) -> f64 {
        self.below(1 << 30) as f64 / (1u64 << 30) as f64
    }
}

/// A note to read: where it is written and which key it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Card {
    pub clef: Clef,
    pub pitch: Pitch,
}

impl Card {
    /// Stable name for its statistics, e.g. "T:C4" for middle C in the treble clef.
    pub fn key(&self) -> String {
        let clef = if self.clef == Clef::Treble { "T" } else { "B" };
        format!("{clef}:{}{}", "CDEFGAB".chars().nth((self.pitch.step % 7) as usize).unwrap(), self.pitch.step / 7)
    }
}

/// Reading levels, following the order notes are introduced in a beginner method.
pub const READING_LEVELS: [&str; 4] = ["Middle C landmarks", "Treble staff", "Bass staff", "Both staves, ledger lines"];

/// The white-key notes practised at a level. Ranges are diatonic steps (C4 = 28).
pub fn note_pool(level: usize) -> Vec<Card> {
    let span = |clef, from: i8, to: i8| (from..=to).map(move |step| Card { clef, pitch: Pitch { step, alter: 0 } });
    match level {
        0 => span(Clef::Treble, 28, 32).chain(span(Clef::Bass, 24, 28)).collect(), // C4–G4, F3–C4
        1 => span(Clef::Treble, 28, 39).collect(),                                   // C4–G5
        2 => span(Clef::Bass, 17, 28).collect(),                                     // F2–C4
        _ => span(Clef::Treble, 26, 42).chain(span(Clef::Bass, 14, 30)).collect(),   // A3–C6, C2–E4
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct CardStats {
    pub seen: u32,
    pub missed: u32,
    /// Recent average time to the right answer, in milliseconds.
    pub ms: f32,
}

impl CardStats {
    /// Records one answer: right or wrong, and how long the right one took.
    pub fn record(&mut self, right: bool, ms: f32) {
        self.seen += 1;
        if right {
            self.ms = if self.ms == 0.0 { ms } else { self.ms * 0.7 + ms * 0.3 };
        } else {
            self.missed += 1;
        }
    }

    /// How much this note needs practice: unseen, missed and slow notes come up more.
    fn need(&self) -> f64 {
        if self.seen == 0 {
            return 3.0;
        }
        let miss_rate = self.missed as f64 / self.seen as f64;
        1.0 + 4.0 * miss_rate + (self.ms as f64 / 1500.0).min(3.0)
    }
}

/// Picks the next card, weighted by need, never repeating the previous one.
pub fn pick(pool: &[Card], stats: &HashMap<String, CardStats>, last: Option<usize>, rng: &mut Rng) -> usize {
    let weights: Vec<f64> =
        pool.iter().enumerate().map(|(i, c)| if Some(i) == last && pool.len() > 1 { 0.0 } else { stats.get(&c.key()).copied().unwrap_or_default().need() }).collect();
    let mut r = rng.unit() * weights.iter().sum::<f64>();
    for (i, w) in weights.iter().enumerate() {
        if r < *w {
            return i;
        }
        r -= w;
    }
    weights.iter().rposition(|w| *w > 0.0).unwrap()
}

/// Interval sizes by name, counted in letter names as a beginner method does (C to E is a 3rd).
pub const INTERVALS: [&str; 7] = ["2nd", "3rd", "4th", "5th", "6th", "7th", "Octave"];
pub const EAR_LEVELS: [&str; 3] = ["2nds and 3rds", "2nds to 5ths", "All intervals to the octave"];

/// A melodic interval on white keys: the two keys to play, and its index in `INTERVALS`.
pub fn interval_question(level: usize, rng: &mut Rng) -> ([u8; 2], usize) {
    let sizes = [2, 4, 7][level.min(2)];
    let size = rng.below(sizes);
    let low = 25 + rng.below(8) as i8; // G3 to D4, so the top note stays in a comfortable range
    let key = |step| Pitch { step, alter: 0 }.midi();
    let (a, b) = (key(low), key(low + size as i8 + 1));
    (if rng.below(2) == 0 { [a, b] } else { [b, a] }, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pools_cover_their_ranges() {
        let names = |level| note_pool(level).iter().map(Card::key).collect::<Vec<_>>();
        assert_eq!(names(0)[..5], ["T:C4", "T:D4", "T:E4", "T:F4", "T:G4"]);
        assert_eq!(names(0)[5], "B:F3");
        assert_eq!(names(1).last().unwrap(), "T:G5");
        assert_eq!(names(2)[0], "B:F2");
        assert!(names(3).len() > 30);
    }

    #[test]
    fn weak_notes_come_up_more_and_never_twice_in_a_row() {
        let pool = note_pool(0);
        let mut stats = HashMap::new();
        for c in &pool {
            stats.insert(c.key(), CardStats { seen: 10, missed: 0, ms: 600.0 });
        }
        stats.insert(pool[3].key(), CardStats { seen: 10, missed: 8, ms: 3000.0 });
        let mut rng = Rng::new(7);
        let mut counts = vec![0; pool.len()];
        let mut last = None;
        for _ in 0..4000 {
            let i = pick(&pool, &stats, last, &mut rng);
            assert_ne!(Some(i), last);
            counts[i] += 1;
            last = Some(i);
        }
        let others = counts.iter().enumerate().filter(|(i, _)| *i != 3).map(|(_, c)| *c).max().unwrap();
        assert!(counts[3] > 2 * others, "the weak note should dominate: {counts:?}");
    }

    #[test]
    fn stats_and_intervals() {
        let mut s = CardStats::default();
        s.record(true, 1000.0);
        s.record(false, 0.0);
        s.record(true, 2000.0);
        assert_eq!((s.seen, s.missed), (3, 1));
        assert_eq!(s.ms, 1300.0);

        let mut rng = Rng::new(3);
        for level in 0..3 {
            for _ in 0..200 {
                let ([a, b], size) = interval_question(level, &mut rng);
                assert!(size < [2, 4, 7][level]);
                // Diatonic step of a white key: octave * 7 + letter.
                let letters = |k: u8| (k / 12) as i32 * 7 + [0, 9, 1, 9, 2, 3, 9, 4, 9, 5, 9, 6][(k % 12) as usize];
                assert!(![a, b].iter().any(|&k| crate::is_black(k)), "white keys only");
                assert_eq!((letters(a) - letters(b)).unsigned_abs() as usize, size + 1);
            }
        }
    }
}

// ---- Generated exercises: fresh sight-reading and rhythm reading every round.

pub const SIGHT_LEVELS: [&str; 5] = ["Right hand", "Left hand", "Hands take turns", "Melody and bass", "F and D major"];
pub const RHYTHM_LEVELS: [&str; 3] = ["Quarters and halves", "With eighths", "With dotted notes"];

/// Note lengths for a bar of 4/4, by difficulty.
const BARS: [&[&[f32]]; 3] = [
    &[&[1.0, 1.0, 1.0, 1.0], &[2.0, 1.0, 1.0], &[1.0, 1.0, 2.0], &[2.0, 2.0], &[1.0, 2.0, 1.0]],
    &[&[1.0, 0.5, 0.5, 1.0, 1.0], &[0.5, 0.5, 1.0, 2.0], &[1.0, 1.0, 0.5, 0.5, 1.0], &[0.5, 0.5, 0.5, 0.5, 2.0]],
    &[&[1.5, 0.5, 2.0], &[3.0, 1.0], &[1.0, 1.5, 0.5, 1.0], &[1.5, 0.5, 1.0, 1.0]],
];

/// A melody walking a five-note position: mostly steps, some skips, a few repeats.
fn walk(rng: &mut Rng, notes: usize) -> Vec<usize> {
    let mut at = [0, 2, 4][rng.below(3)];
    (0..notes)
        .map(|i| {
            if i > 0 {
                let mv: i32 = match rng.below(10) {
                    0..=5 => [-1, 1][rng.below(2)],
                    6..=8 => [-2, 2][rng.below(2)],
                    _ => 0,
                };
                // Bounce off the edges of the hand.
                let next = at as i32 + mv;
                at = if (0..5).contains(&next) { next as usize } else { (at as i32 - mv).clamp(0, 4) as usize };
            }
            at
        })
        .collect()
}

/// Four bars of rhythm: easy bars, ending on a long note.
fn bars(rng: &mut Rng, level: usize) -> Vec<f32> {
    let mut beats = Vec::new();
    for bar in 0..4 {
        let pool: Vec<&[f32]> = (0..=level).flat_map(|l| BARS[l].iter().copied()).collect();
        let pattern: &[f32] = if bar == 3 { [&[2.0, 2.0][..], &[4.0][..]][rng.below(2)] } else { pool[rng.below(pool.len())] };
        beats.extend_from_slice(pattern);
    }
    beats
}

fn token(note: &str, beats: f32) -> String {
    format!("{note}/{beats}")
}

/// One sight-reading passage as (rh, lh) part texts.
fn passage(level: usize, rng: &mut Rng) -> (String, String) {
    const C_RH: [&str; 5] = ["C4", "D4", "E4", "F4", "G4"];
    const C_LH: [&str; 5] = ["C3", "D3", "E3", "F3", "G3"];
    const G_RH: [&str; 5] = ["G4", "A4", "B4", "C5", "D5"];
    let rhythm = bars(rng, 0);
    let steps = walk(rng, rhythm.len());
    let line = |names: &[&str; 5], finger: &dyn Fn(usize) -> usize| -> String {
        let mut out: Vec<String> = steps.iter().zip(&rhythm).map(|(&s, &b)| token(names[s], b)).collect();
        out[0] = format!("{}@{}", out[0], finger(steps[0])); // the starting finger, as books print it
        out.join(" ")
    };
    match level {
        0 => (line(&C_RH, &|s| s + 1), String::new()),
        1 => (String::new(), line(&C_LH, &|s| 5 - s)),
        2 => {
            // Bars alternate hands; the resting hand counts a whole-bar rest.
            let (mut rh, mut lh, mut i) = (Vec::new(), Vec::new(), 0);
            for bar in 0..4 {
                let mut used = 0.0;
                let (play, rest) = if bar % 2 == 0 { (&mut rh, &mut lh) } else { (&mut lh, &mut rh) };
                let names = if bar % 2 == 0 { &C_RH } else { &C_LH };
                while used < 4.0 {
                    play.push(token(names[steps[i]], rhythm[i]));
                    used += rhythm[i];
                    i += 1;
                }
                rest.push("r/4".to_string());
            }
            (rh.join(" "), lh.join(" "))
        }
        3 => {
            (line(&G_RH, &|s| s + 1), "G2/4@5 D3/4 G2/4 D3/4".to_string())
        }
        _ => {
            // F major (one flat) or D major (two sharps), with its key signature and a root–fifth bass.
            const F_RH: [&str; 5] = ["F4", "G4", "A4", "Bb4", "C5"];
            const D_RH: [&str; 5] = ["D4", "E4", "F#4", "G4", "A4"];
            let (names, key, [root, fifth]) = if rng.below(2) == 0 { (&F_RH, -1, ["F2", "C3"]) } else { (&D_RH, 2, ["D2", "A2"]) };
            (format!("key={key} {}", line(names, &|s| s + 1)), format!("key={key} {root}/4@5 {fifth}/4 {root}/4 {fifth}/4"))
        }
    }
}

/// A round of three fresh sight-reading passages, as a lesson.
pub fn sight_reading(level: usize, rng: &mut Rng) -> crate::lesson::Lesson {
    let level = level.min(SIGHT_LEVELS.len() - 1);
    let steps: Vec<String> = (1..=3)
        .map(|k| {
            let (rh, lh) = passage(level, rng);
            format!(
                r#"Play(prompt: "Passage {k} of 3. Find the starting note, then read ahead and keep going.", rh: "{rh}", lh: "{lh}", beats_per_bar: 4, bpm: 72, tips: ["Before playing, scan it: where does it go up, down, skip?", "Look one note ahead of the one you are playing.", "A wrong note is fine; stopping is not. Try In time once it reads easily."])"#
            )
        })
        .collect();
    let source = format!(r#"(id: "practice-sight", title: "Sight-reading · {}", summary: "Three new passages", steps: [{}])"#, SIGHT_LEVELS[level], steps.join(", "));
    crate::lesson::Lesson::parse(&source).expect("generated sight-reading is valid")
}

/// A round of four fresh two-bar rhythms to tap, as a lesson.
pub fn rhythm_reading(level: usize, rng: &mut Rng) -> crate::lesson::Lesson {
    let level = level.min(RHYTHM_LEVELS.len() - 1);
    let steps: Vec<String> = (1..=4)
        .map(|k| {
            let beats: Vec<String> = bars(rng, level)[..].iter().map(|b| format!("{b:?}")).collect();
            // Two bars: take the first notes that fill eight beats.
            let mut sum = 0.0;
            let two: Vec<String> = beats.into_iter().take_while(|b| { let keep = sum < 8.0; sum += b.parse::<f32>().unwrap(); keep }).collect();
            format!(r#"PlayRhythm(prompt: "Rhythm {k} of 4. Count the bar in, then tap.", bpm: 72, beats_per_bar: 4, beats: [{}])"#, two.join(", "))
        })
        .collect();
    let source = format!(r#"(id: "practice-rhythm", title: "Rhythm reading · {}", summary: "Four new rhythms", steps: [{}])"#, RHYTHM_LEVELS[level], steps.join(", "));
    crate::lesson::Lesson::parse(&source).expect("generated rhythm reading is valid")
}

pub const CHORD_LEVELS: [&str; 4] = ["C, F and G", "All major chords", "Major and minor", "Inversions"];

/// A round of eight chords to find in root position, starting in Middle C's octave.
pub fn chord_finding(level: usize, rng: &mut Rng) -> crate::lesson::Lesson {
    const ROOTS: [&str; 12] = ["C", "D♭", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B"];
    const SPELL: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let level = level.min(CHORD_LEVELS.len() - 1);
    let key = |m: u8| format!("{}{}", SPELL[(m % 12) as usize], m / 12 - 1);
    let mut steps = vec![r#"Explain(text: "A major chord: the root, up four half steps, then up three more. A minor chord swaps them: three, then four. Play each chord with its root in Middle C's octave.", show: Keys((from: "C4", to: "B4", marks: [("C4", "root"), ("E4", "+4"), ("G4", "+3")], arcs: [("C4", "E4", "4 half steps"), ("E4", "G4", "3")])))"#.to_string()];
    let mut last = 99;
    for _ in 0..8 {
        let (root, minor) = loop {
            let root = if level == 0 { [0, 5, 7][rng.below(3)] } else { rng.below(12) };
            let minor = level >= 2 && rng.below(2) == 0;
            if root != last {
                break (root, minor);
            }
        };
        last = root;
        let base = 60 + root as u8;
        let third = base + if minor { 3 } else { 4 };
        let name = format!("{} {}", ROOTS[root], if minor { "minor" } else { "major" });
        // Inversions move the bottom note up an octave: the 3rd or the 5th ends up lowest.
        let (keys, how) = match if level == 3 { rng.below(3) } else { 0 } {
            0 => ([base, third, base + 7], String::new()),
            1 => ([third, base + 7, base + 12], ", first inversion: the 3rd at the bottom".to_string()),
            _ => ([base + 7, base + 12, third + 12], ", second inversion: the 5th at the bottom".to_string()),
        };
        steps.push(format!(r#"Find(prompt: "Play {name}{how}.", keys: ["{}", "{}", "{}"])"#, key(keys[0]), key(keys[1]), key(keys[2])));
    }
    let source = format!(r#"(id: "practice-chords", title: "Chord finding · {}", summary: "Eight chords to find", steps: [{}])"#, CHORD_LEVELS[level], steps.join(", "));
    crate::lesson::Lesson::parse(&source).expect("generated chord round is valid")
}

#[cfg(test)]
mod generated {
    use super::*;
    use crate::lesson::Step;

    #[test]
    fn every_level_generates_valid_full_bars() {
        let mut rng = Rng::new(11);
        let mut keys_seen = std::collections::HashSet::new();
        for _ in 0..200 {
            for level in 0..SIGHT_LEVELS.len() {
                let lesson = sight_reading(level, &mut rng);
                assert_eq!(lesson.steps.len(), 3);
                for step in &lesson.steps {
                    let music = step.music().unwrap();
                    keys_seen.insert(music.key);
                    for hand in [crate::music::Hand::Right, crate::music::Hand::Left] {
                        let total: f32 = music.events.iter().filter(|e| e.hand == hand).map(|e| e.beats).sum();
                        assert!(total == 0.0 || total == 16.0, "level {level}: a hand fills exactly four bars, got {total}");
                    }
                }
            }
            for level in 0..RHYTHM_LEVELS.len() {
                for step in rhythm_reading(level, &mut rng).steps {
                    let Step::PlayRhythm { beats, .. } = step else { panic!() };
                    assert_eq!(beats.iter().sum::<f32>(), 8.0, "two full bars");
                }
            }
        }
        assert!(keys_seen.contains(&-1) && keys_seen.contains(&2), "the new-keys level visits F and D major: {keys_seen:?}");
        for level in (0..CHORD_LEVELS.len()).flat_map(|l| std::iter::repeat_n(l, 50)) {
            for step in chord_finding(level, &mut rng).steps.iter().skip(1) {
                let Step::Find { keys, prompt, .. } = step else { panic!() };
                let k: Vec<u8> = keys.iter().map(|p| p.midi()).collect();
                let minor = prompt.contains("minor");
                let (third, fifth) = (if minor { 3 } else { 4 }, 7);
                let expect = if prompt.contains("first inversion") {
                    (fifth - third, 12 - third)
                } else if prompt.contains("second inversion") {
                    (12 - fifth, 12 - fifth + third)
                } else {
                    (third, fifth)
                };
                assert_eq!((k[1] - k[0], k[2] - k[0]), expect, "{prompt}: the right notes, stacked in order");
                assert!((60..80).contains(&k[0]));
                if level == 0 {
                    assert!([60, 65, 67].contains(&k[0]), "{prompt}");
                }
            }
        }
    }
}
