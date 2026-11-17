//! Lesson file format (RON) and the state machine that walks a learner through one.

use serde::Deserialize;

use crate::{
    Pitch,
    music::{Music, Part},
};

#[derive(Clone, Debug, Deserialize)]
pub struct Lesson {
    pub id: String,
    pub title: String,
    pub summary: String,
    /// Where the music comes from, e.g. "Hanon, The Virtuoso Pianist, No. 1".
    #[serde(default)]
    pub source: String,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Deserialize)]
pub enum Step {
    Explain {
        text: String,
        #[serde(default)]
        show: Show,
    },
    Quiz {
        question: String,
        #[serde(default)]
        show: Show,
        choices: Vec<String>,
        answer: usize,
    },
    /// Find every one of these keys on the keyboard, in any order.
    Find {
        prompt: String,
        keys: Vec<Pitch>,
        #[serde(default)]
        show: Show,
    },
    /// Play the music at your own pace: each note or chord waits until all its keys are pressed.
    Play {
        prompt: String,
        #[serde(default)]
        rh: Part,
        #[serde(default)]
        lh: Part,
        /// Barlines and a time signature every this many beats; 0 for neither.
        #[serde(default)]
        beats_per_bar: u8,
        /// Mark the next keys on the on-screen keyboard.
        #[serde(default)]
        hint: bool,
        /// Print the letter name inside each notehead.
        #[serde(default)]
        names: bool,
        /// Tempo for the Listen button.
        #[serde(default = "default_bpm")]
        bpm: f32,
        /// Play it back by ear: the notation stays hidden until the passage is played right.
        #[serde(default)]
        hide: bool,
        /// Coaching points shown beside the music: how to play it well.
        #[serde(default)]
        tips: Vec<String>,
    },
    /// Tap the rhythm on any key, in time with a metronome. Note lengths are in quarter-note beats.
    PlayRhythm {
        prompt: String,
        bpm: f32,
        beats_per_bar: u8,
        beats: Vec<f32>,
        /// How far from the beat a tap may land and still count as in time.
        #[serde(default = "default_tolerance")]
        tolerance_ms: f32,
    },
}

fn default_bpm() -> f32 {
    80.0
}

fn default_tolerance() -> f32 {
    100.0
}

#[derive(Clone, Debug, Default, Deserialize)]
pub enum Show {
    #[default]
    Nothing,
    Keyboard(Vec<Pitch>),
    /// Notes on one staff, drawn as whole notes: pitch only, no rhythm implied.
    Staff(Clef, Vec<Pitch>),
    /// Music for the right and left hands, on a grand staff when both are given.
    Music(Part, Part),
    /// A labelled keyboard diagram.
    Keys(KeyDiagram),
    /// Both hands with their finger numbers; the listed fingers are highlighted.
    Hands(Vec<u8>),
    /// A drawn illustration: "posture", "note-values", "dynamics", "pulse" or "listen".
    Figure(String),
}

pub const FIGURES: [&str; 5] = ["posture", "note-values", "dynamics", "pulse", "listen"];

/// A stretch of keyboard drawn large, with labels on keys and arcs between them.
#[derive(Clone, Debug, Deserialize)]
pub struct KeyDiagram {
    pub from: Pitch,
    pub to: Pitch,
    /// A label on a key; the key is highlighted.
    #[serde(default)]
    pub marks: Vec<(Pitch, String)>,
    /// An arc from one key to another, with a label above it.
    #[serde(default)]
    pub arcs: Vec<(Pitch, Pitch, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub enum Clef {
    Treble,
    Bass,
}

impl Step {
    /// The music of a Play step.
    pub fn music(&self) -> Option<Music> {
        match self {
            Step::Play { rh, lh, .. } => Some(Music::new(rh, lh)),
            _ => None,
        }
    }
}

impl Show {
    fn problem(&self) -> Option<&'static str> {
        match self {
            Show::Keys(d) if d.from.midi() >= d.to.midi() || d.to.midi() - d.from.midi() > 48 => Some("a key diagram spans 1 to 48 keys, low to high"),
            Show::Keys(d) if d.marks.iter().map(|m| &m.0).chain(d.arcs.iter().flat_map(|a| [&a.0, &a.1])).any(|p| p.midi() < d.from.midi() || p.midi() > d.to.midi()) => {
                Some("a key diagram marks a key outside its range")
            }
            Show::Hands(f) if f.iter().any(|f| !(1..=5).contains(f)) => Some("fingers are 1 to 5"),
            Show::Figure(name) if !FIGURES.contains(&name.as_str()) => Some("unknown figure"),
            _ => None,
        }
    }
}

impl Lesson {
    /// Steps that can be played against the metronome: passages (not by-ear ones) and rhythms.
    pub fn timed_steps(&self) -> Vec<usize> {
        (0..self.steps.len()).filter(|&i| matches!(self.steps[i], Step::Play { hide: false, .. } | Step::PlayRhythm { .. })).collect()
    }

    /// Parses and validates a lesson file; lesson files are the trust boundary for content.
    pub fn parse(source: &str) -> Result<Lesson, String> {
        let lesson: Lesson = ron::from_str(source).map_err(|e| e.to_string())?;
        if lesson.steps.is_empty() {
            return Err("lesson has no steps".into());
        }
        for (i, step) in lesson.steps.iter().enumerate() {
            let show = match step {
                Step::Explain { show, .. } | Step::Quiz { show, .. } | Step::Find { show, .. } => show.problem(),
                _ => None,
            };
            if let Some(problem) = show {
                return Err(format!("step {}: {problem}", i + 1));
            }
            let problem = match step {
                Step::Find { keys, .. } if keys.is_empty() => "no keys to find",
                Step::Quiz { choices, answer, .. } if *answer >= choices.len() => "answer is not one of the choices",
                Step::Play { .. } if !step.music().unwrap().has_notes() => "no notes to play",
                Step::Play { beats_per_bar, .. } if *beats_per_bar == 1 || *beats_per_bar > 6 => "beats_per_bar must be 0 (none) or 2 to 6",
                Step::Play { bpm, .. } if !(30.0..=200.0).contains(bpm) => "bpm must be between 30 and 200",
                Step::PlayRhythm { beats, .. } if beats.is_empty() => "no rhythm to tap",
                Step::PlayRhythm { beats, .. } if beats.iter().any(|b| ![0.5, 1.0, 1.5, 2.0, 3.0, 4.0].contains(b)) => {
                    "note lengths must be 0.5, 1, 1.5, 2, 3 or 4 beats"
                }
                Step::PlayRhythm { bpm, .. } if !(30.0..=200.0).contains(bpm) => "bpm must be between 30 and 200",
                Step::PlayRhythm { beats_per_bar, .. } if !(2..=6).contains(beats_per_bar) => "beats_per_bar must be between 2 and 6",
                Step::PlayRhythm { tolerance_ms, .. } if !(20.0..=300.0).contains(tolerance_ms) => "tolerance_ms must be between 20 and 300",
                _ => continue,
            };
            return Err(format!("step {}: {problem}", i + 1));
        }
        Ok(lesson)
    }
}

/// One run through a lesson.
pub struct Session {
    pub lesson: Lesson,
    pub step: usize,
    /// The current step's music and its moments (event indices), for Play steps.
    music: Music,
    moments: Vec<Vec<usize>>,
    /// The keys to press, group by group: one group per Play moment, or one for a Find step.
    targets: Vec<Vec<u8>>,
    /// Groups finished so far in the current step.
    pub moment: usize,
    /// Keys of the current group already pressed.
    pressed: Vec<u8>,
    /// The current quiz or exercise has been answered correctly.
    pub solved: bool,
}

impl Session {
    pub fn new(lesson: Lesson) -> Self {
        let mut s = Session { lesson, step: 0, music: Music::default(), moments: Vec::new(), targets: Vec::new(), moment: 0, pressed: Vec::new(), solved: false };
        s.enter();
        s
    }

    fn enter(&mut self) {
        self.music = self.current().and_then(Step::music).unwrap_or_default();
        self.moments = self.music.moments();
        self.targets = match self.current() {
            Some(Step::Find { keys, .. }) => {
                let mut keys: Vec<u8> = keys.iter().map(|p| p.midi()).collect();
                keys.sort_unstable();
                keys.dedup();
                vec![keys]
            }
            _ => self.moments.iter().map(|m| self.music.keys(m)).collect(),
        };
        self.moment = 0;
        self.pressed.clear();
        self.solved = false;
    }

    /// The current step, or `None` once the lesson is finished.
    pub fn current(&self) -> Option<&Step> {
        self.lesson.steps.get(self.step)
    }

    /// Keys still to press for the current group; empty when no exercise is waiting.
    pub fn target(&self) -> Vec<u8> {
        let Some(keys) = self.targets.get(self.moment) else { return Vec::new() };
        keys.iter().copied().filter(|k| !self.pressed.contains(k)).collect()
    }

    /// The keys of each group to press, in order.
    pub fn targets(&self) -> &[Vec<u8>] {
        &self.targets
    }

    /// The events (indices into the step's music) of each group, for Play steps.
    pub fn moments(&self) -> &[Vec<usize>] {
        &self.moments
    }

    /// The current step's music: both hands, in time order.
    pub fn music(&self) -> &Music {
        &self.music
    }

    /// Keys of the current group already found.
    pub fn found(&self) -> &[u8] {
        &self.pressed
    }

    /// A key was pressed. `None` if no exercise is waiting or the key was already held for this
    /// moment, otherwise whether it belongs to the moment.
    pub fn note_on(&mut self, note: u8) -> Option<bool> {
        let target = self.target();
        if target.is_empty() || self.pressed.contains(&note) {
            return None;
        }
        if !target.contains(&note) {
            return Some(false);
        }
        self.pressed.push(note);
        if target.len() == 1 {
            self.moment += 1;
            self.solved = self.moment == self.targets.len();
            if !self.solved {
                self.pressed.clear(); // a finished Find step keeps its found keys on show
            }
        }
        Some(true)
    }

    /// Per event of the current music: 0 ahead, 1 played, 3 to play now.
    pub fn marks(&self) -> Vec<i32> {
        let mut marks = vec![0; self.music.events.len()];
        for (m, moment) in self.moments.iter().enumerate() {
            for &i in moment {
                let all_pressed = self.music.events[i].pitches.iter().all(|p| self.pressed.contains(&p.midi()));
                marks[i] = if m < self.moment || (m == self.moment && all_pressed) {
                    1
                } else if m == self.moment {
                    3
                } else {
                    0
                };
            }
        }
        marks
    }

    /// A quiz choice was picked. `None` if no quiz is waiting.
    pub fn answer(&mut self, choice: usize) -> Option<bool> {
        match self.current()? {
            Step::Quiz { answer, .. } if !self.solved => {
                self.solved = choice == *answer;
                Some(self.solved)
            }
            _ => None,
        }
    }

    pub fn can_advance(&self) -> bool {
        matches!(self.current(), Some(Step::Explain { .. })) || self.solved
    }

    /// Goes straight to a step (the last one if out of range), starting it afresh.
    pub fn jump(&mut self, step: usize) {
        self.step = step.min(self.lesson.steps.len() - 1);
        self.enter();
    }

    /// Starts the current step again from its beginning.
    pub fn restart(&mut self) {
        self.enter();
    }

    /// Goes back to the previous step, starting it afresh.
    pub fn back(&mut self) {
        if self.step > 0 {
            self.step -= 1;
            self.enter();
        }
    }

    /// Moves to the next step if allowed. Returns true when that finished the lesson.
    pub fn advance(&mut self) -> bool {
        if self.can_advance() {
            self.step += 1;
            self.enter();
        }
        self.current().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"(
        id: "t", title: "T", summary: "S",
        steps: [
            Explain(text: "hello", show: Keyboard(["C4"])),
            Quiz(question: "?", choices: ["a", "b"], answer: 1),
            Play(prompt: "play", rh: "C4 D4"),
            Play(prompt: "both", rh: "E4/2", lh: "C3+G3/2"),
            Find(prompt: "find", keys: ["C4", "C5"], show: Keys((from: "C4", to: "C5", marks: [("C4", "C")], arcs: [("C4", "C5", "octave")]))),
        ],
    )"#;

    #[test]
    fn walks_through_a_lesson() {
        let mut s = Session::new(Lesson::parse(SAMPLE).unwrap());
        assert_eq!(s.note_on(60), None, "keys are free play during an explanation");
        assert!(!s.advance());

        assert!(!s.can_advance());
        assert_eq!(s.answer(0), Some(false));
        assert!(!s.advance() && s.step == 1, "a wrong answer does not unlock the next step");
        assert_eq!(s.answer(1), Some(true));
        assert_eq!(s.answer(0), None, "a solved quiz ignores further picks");
        assert!(!s.advance());

        s.back();
        assert_eq!((s.step, s.solved), (1, false), "going back restarts the quiz");
        s.jump(99);
        assert_eq!(s.step, 4, "jumping past the end lands on the last step");
        s.jump(1);
        assert_eq!(s.answer(1), Some(true));
        assert!(!s.advance());

        assert_eq!(s.target(), [60]);
        assert_eq!(s.note_on(62), Some(false));
        assert_eq!(s.note_on(60), Some(true));
        assert_eq!(s.marks(), [1, 3]);
        assert!(!s.can_advance());
        assert_eq!(s.note_on(62), Some(true));
        assert!(s.solved);
        assert!(!s.advance());

        // A chord across both hands: any order, each key once.
        assert_eq!(s.target(), [48, 55, 64]);
        assert_eq!(s.note_on(55), Some(true));
        assert_eq!(s.note_on(55), None, "a key already held for this chord is ignored");
        assert_eq!(s.note_on(64), Some(true));
        assert!(!s.solved);
        assert_eq!(s.note_on(48), Some(true));
        assert!(!s.advance());

        assert_eq!(s.target(), [60, 72]);
        assert_eq!(s.note_on(72), Some(true));
        assert_eq!(s.found(), [72]);
        assert_eq!(s.note_on(60), Some(true));
        assert_eq!(s.found(), [72, 60], "found keys stay on show once the step is solved");
        assert!(s.advance(), "last step done finishes the lesson");
    }

    #[test]
    fn rejects_broken_lessons() {
        assert!(Lesson::parse(r#"(id: "t", title: "T", summary: "S", steps: [])"#).is_err());
        assert!(Lesson::parse(&SAMPLE.replace("answer: 1", "answer: 2")).is_err());
        assert!(Lesson::parse(&SAMPLE.replace(r#"rh: "C4 D4""#, r#"rh: "r/4""#)).is_err());
        assert!(Lesson::parse(&SAMPLE.replace("D4", "X4")).is_err());
        assert!(Lesson::parse(&SAMPLE.replace(r#"marks: [("C4", "C")]"#, r#"marks: [("D5", "D")]"#)).unwrap_err().contains("outside its range"));
        assert!(Lesson::parse(&SAMPLE.replace(r#"keys: ["C4", "C5"]"#, "keys: []")).is_err());
        assert!(Lesson::parse(&SAMPLE.replace(r#"show: Keyboard(["C4"])"#, r#"show: Figure("nope")"#)).unwrap_err().contains("unknown figure"));
        let rhythm = |fields: &str| Lesson::parse(&format!(r#"(id: "t", title: "T", summary: "S", steps: [PlayRhythm(prompt: "tap", {fields})])"#));
        assert!(rhythm("bpm: 80, beats_per_bar: 4, beats: [1.0, 0.5, 0.5, 2.0]").is_ok());
        assert!(rhythm("bpm: 80, beats_per_bar: 4, beats: []").is_err());
        assert!(rhythm("bpm: 80, beats_per_bar: 4, beats: [0.25]").is_err());
        assert!(rhythm("bpm: 0, beats_per_bar: 4, beats: [1.0]").is_err());
        assert!(rhythm("bpm: 80, beats_per_bar: 9, beats: [1.0]").is_err());
    }

    #[test]
    fn every_shipped_passage_fills_whole_bars() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/lessons");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let lesson = Lesson::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
            for (i, step) in lesson.steps.iter().enumerate() {
                let Step::Play { beats_per_bar, tips, hide, .. } = step else { continue };
                assert!(*hide || !tips.is_empty(), "{} step {}: every passage to play needs coaching tips", path.display(), i + 1);
                let music = step.music().unwrap();
                let length = |hand| music.events.iter().filter(|e| e.hand == hand).map(|e| e.beats).sum::<f32>();
                let (rh, lh) = (length(crate::music::Hand::Right), length(crate::music::Hand::Left));
                let at = format!("{} step {}", path.display(), i + 1);
                if *beats_per_bar > 0 {
                    for len in [rh, lh].into_iter().filter(|&l| l > 0.0) {
                        assert_eq!(len % *beats_per_bar as f32, 0.0, "{at}: a hand ends partway through a bar ({len} beats)");
                    }
                }
                assert!(rh == 0.0 || lh == 0.0 || rh == lh, "{at}: the hands differ in length ({rh} and {lh} beats)");
            }
        }
    }

    #[test]
    fn every_shipped_lesson_parses() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/lessons");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let source = std::fs::read_to_string(&path).unwrap();
            if let Err(e) = Lesson::parse(&source) {
                panic!("{}: {e}", path.display());
            }
            count += 1;
        }
        assert!(count > 0, "no lessons found in {dir}");
    }
}
