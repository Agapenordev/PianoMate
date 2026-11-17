mod audio;

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    error::Error,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use audio::{Audio, Msg, now_ms};
use pianomate_core::{
    QWERTY,
    curriculum::{Curriculum, Goal, Stage, emblem},
    drill::{self, Card as NoteCard, CardStats, EAR_LEVELS, INTERVALS, READING_LEVELS, CHORD_LEVELS, RHYTHM_LEVELS, Rng, SIGHT_LEVELS},
    is_black,
    lesson::{KeyDiagram, Lesson, Session, Show, Step},
    music::Music,
    notation::{self, Layout, Shape},
    note_name, qwerty_note,
    rhythm::{Mark, RhythmRun, median_offset},
};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

slint::include_modules!();

const LOW: u8 = 21; // A0
const HIGH: u8 = 108; // C8
const CALIBRATION_BPM: f32 = 100.0;
const CALIBRATION_TAPS: usize = 8;
const READING_ROUND: usize = 20;
const EAR_ROUND: usize = 10;
/// Gaps between key presses shorter than this count as practice time.
const PRACTICE_GAP_MS: f64 = 30_000.0;
// shortcut: content is read from the working directory; embed it when the app is packaged.
const CONTENT: &str = "content";

/// What is kept between runs.
#[derive(Default, Serialize, Deserialize)]
struct Save {
    /// Ids of completed lessons.
    done: BTreeSet<String>,
    /// How long after a click this player's taps arrive on this device; `None` until calibrated.
    #[serde(default)]
    latency_ms: Option<f64>,
    /// Id of the chosen goal; `None` until the first run's goal picker is answered.
    #[serde(default)]
    goal: Option<String>,
    /// Days (counted from 1970) on which a lesson was completed, for the streak.
    #[serde(default)]
    days: BTreeSet<u64>,
    /// 0 follow the system, 1 light, 2 dark.
    #[serde(default)]
    theme: i32,
    /// Letter names inside every notehead.
    #[serde(default)]
    names: bool,
    /// Seconds of playing per day.
    #[serde(default)]
    practice: BTreeMap<u64, u32>,
    /// The last day each kind of practice was done: "reading", "lesson", "ear".
    #[serde(default)]
    activity: BTreeMap<String, u64>,
    /// Note-reading statistics per note (`Card::key`).
    #[serde(default)]
    cards: HashMap<String, CardStats>,
    #[serde(default)]
    reading_level: usize,
    #[serde(default)]
    ear_level: usize,
    /// Ear training answers per level: (right first time, asked).
    #[serde(default)]
    ear: BTreeMap<usize, (u32, u32)>,
    /// Play PianoMate's piano sound for notes from a MIDI keyboard (off for keyboards with speakers).
    #[serde(default)]
    echo_midi: bool,
    #[serde(default)]
    sight_level: usize,
    #[serde(default)]
    rhythm_level: usize,
    #[serde(default)]
    chord_level: usize,
    /// Generated practice rounds finished, by kind ("sight", "rhythm").
    #[serde(default)]
    rounds: BTreeMap<String, u32>,
    /// Best tempo (percent) at which each timed step was passed In time, by "lesson-id:step".
    #[serde(default)]
    in_time: BTreeMap<String, i32>,
    /// The practice metronome; 0 means not yet set (80 bpm, 4 beats).
    #[serde(default)]
    metronome_bpm: i32,
    #[serde(default)]
    metronome_beats: i32,
    /// Show the on-screen keyboard along the bottom (hidden until turned on in Setup).
    #[serde(default)]
    show_keyboard: bool,
    /// The lesson (id) and step the learner was last on, to continue from exactly there.
    #[serde(default)]
    resume: Option<(String, usize)>,
    /// How much slower than written to play back and play along, in percent (0 = as written).
    #[serde(default)]
    slowdown: i32,
}

impl Save {
    /// Playback and play-along speed as a fraction of the written tempo.
    fn tempo(&self) -> f32 {
        (100 - self.slowdown.clamp(0, 60)) as f32 / 100.0
    }
}

fn today() -> u64 {
    // shortcut: days roll over at midnight UTC, not local time; use a time-zone crate if streaks feel wrong.
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400)
}

fn main() -> Result<(), Box<dyn Error>> {
    let sf_path = std::env::args().nth(1).unwrap_or("assets/piano.sf2".into());
    let lessons = load_lessons(&format!("{CONTENT}/lessons"))?;
    let ids: Vec<&str> = lessons.iter().map(|l| l.id.as_str()).collect();
    let curriculum_path = format!("{CONTENT}/curriculum.ron");
    let curriculum = fs::read_to_string(&curriculum_path)
        .map_err(|e| e.to_string())
        .and_then(|text| Curriculum::parse(&text, &ids))
        .map_err(|e| format!("{curriculum_path}: {e}"))?;
    let audio = Audio::start(&sf_path)?;
    let send = audio.sender();

    let app = App::new()?;
    // A stable app-id lets window managers recognise PianoMate (e.g. to exempt it from transparency
    // rules). It must be set after the platform starts and before the window is shown.
    slint::set_xdg_app_id("pianomate")?;
    app.set_keys(ModelRc::new(VecModel::from(build_keys())));
    app.set_audio_info(audio.info.as_str().into());
    app.set_reading_levels(strings(&READING_LEVELS));
    app.set_ear_levels(strings(&EAR_LEVELS));
    app.set_sight_levels(strings(&SIGHT_LEVELS));
    app.set_rhythm_levels(strings(&RHYTHM_LEVELS));
    app.set_chord_levels(strings(&CHORD_LEVELS));

    let save_path = directories::ProjectDirs::from("", "", "PianoMate")
        .ok_or("no home directory to keep progress in")?
        .data_dir()
        .join("progress.ron");
    let save = match fs::read_to_string(&save_path) {
        Ok(text) => ron::from_str(&text)
            // Files from the first versions are just the list of finished lessons.
            .or_else(|_| ron::from_str(&text).map(|done| Save { done, ..Default::default() }))
            .unwrap_or_else(|e| {
                eprintln!("ignoring unreadable progress file {}: {e}", save_path.display());
                Save::default()
            }),
        Err(_) => Save::default(),
    };
    app.global::<Theme>().set_mode(save.theme);
    let save_echo = save.echo_midi;

    let ctl = Rc::new(RefCell::new(Ctl {
        app: app.as_weak(),
        audio,
        timer: Timer::default(),
        lessons,
        curriculum,
        images: RefCell::new(HashMap::new()),
        save,
        save_path,
        page: 0,
        stage_open: 0,
        choosing_goal: false,
        session: None,
        choice_states: Vec::new(),
        choice_order: Vec::new(),
        feedback: (String::new(), true),
        rhythm: None,
        calibration: None,
        drill: None,
        echo: Arc::new(AtomicBool::new(save_echo)),
        in_time: false,
        metronome_on: false,
        listening: None,
        listen_generation: 0,
        shown_step: std::cell::Cell::new(None),
        rng: Rng::new(SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64)),
        last_key_ms: None,
    }));
    {
        // Drives the playhead and the end of a rhythm run; only runs while one is in progress.
        let (weak, ctl) = (Rc::downgrade(&ctl), ctl.borrow());
        ctl.timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            if let Some(ctl) = weak.upgrade() {
                ctl.borrow_mut().tick();
            }
        });
        ctl.timer.stop();
        ctl.show();
    }

    // On-screen keys and the computer keyboard make sound here; MIDI keys sound from their own thread.
    // Both then report to `key-played`, which lights the key and grades it.
    app.on_note({
        let (weak, send) = (app.as_weak(), send.clone());
        move |note, down| {
            let at = now_ms();
            send(Msg::Midi([if down { 0x90 } else { 0x80 }, note as u8, 100]));
            if let Some(app) = weak.upgrade() {
                app.invoke_key_played(note, down, at as i32);
            }
        }
    });
    app.on_key_played({
        let ctl = ctl.clone();
        move |note, down, at| ctl.borrow_mut().played(note as u8, down, at as f64)
    });
    app.on_typed({
        let (weak, ctl) = (app.as_weak(), ctl.clone());
        move |text, down| {
            let (Some(c), Some(app)) = (text.chars().next(), weak.upgrade()) else { return };
            if let Some(note) = qwerty_note(c) {
                app.invoke_note(note as i32, down);
            } else if let (Some(n @ 1..=9), true) = (c.to_digit(10), down) {
                ctl.borrow_mut().choose(n as usize - 1);
            } else if c == ' ' && down {
                ctl.borrow_mut().replay();
            }
        }
    });
    // Each UI callback forwards to the method of the same name.
    macro_rules! forward {
        ($on:ident => $method:ident($($arg:ident),*)) => {
            app.$on({
                let ctl = ctl.clone();
                move |$($arg),*| ctl.borrow_mut().$method($($arg as _),*)
            });
        };
    }
    forward!(on_navigate => navigate(page));
    forward!(on_pick_goal => pick_goal(index));
    forward!(on_change_goal => change_goal());
    forward!(on_open_stage => open_stage(index));
    forward!(on_open_part_stage => open_part_stage(index));
    forward!(on_open_lesson => open_stage_lesson(index));
    forward!(on_plan_item => plan_item(kind));
    forward!(on_calibrate => calibrate());
    forward!(on_set_theme => set_theme(mode));
    forward!(on_toggle_names => toggle_names());
    forward!(on_toggle_echo => toggle_echo());
    forward!(on_toggle_piano => toggle_piano());
    forward!(on_pick_reading_level => pick_reading_level(index));
    forward!(on_pick_ear_level => pick_ear_level(index));
    forward!(on_start_reading => start_reading());
    forward!(on_start_ear => start_ear());
    forward!(on_start_sight => start_sight());
    forward!(on_start_chords => start_chords());
    forward!(on_metronome_toggle => metronome_toggle());
    forward!(on_metronome_bpm_by => metronome_bpm_by(delta));
    forward!(on_metronome_beats_set => metronome_beats_set(beats));
    forward!(on_pick_chord_level => pick_chord_level(index));
    forward!(on_start_rhythm_reading => start_rhythm_reading());
    forward!(on_pick_sight_level => pick_sight_level(index));
    forward!(on_pick_rhythm_level => pick_rhythm_level(index));
    forward!(on_replay => replay());
    forward!(on_listen => listen());
    forward!(on_previous => previous());
    forward!(on_retry => retry());
    forward!(on_set_mode => set_mode(mode));
    forward!(on_set_tempo => set_tempo(percent));
    forward!(on_listen_tick => listen_tick(generation, event, on));
    forward!(on_home => home());
    forward!(on_next => next());
    forward!(on_choose => choose(index));

    // MIDI keyboards: connected at start and whenever one is plugged in or out.
    let weak = app.as_weak();
    let echo = ctl.borrow().echo.clone();
    let midi = Rc::new(RefCell::new(audio::Midi::new(
        // A keyboard with its own speakers should not be doubled by PianoMate's sound.
        move |msg| {
            if echo.load(Ordering::Relaxed) {
                send(msg);
            }
        },
        move |note, down, at| {
            let _ = weak.upgrade_in_event_loop(move |app| app.invoke_key_played(note as i32, down, at as i32));
        },
    )));
    let show_devices = {
        let (weak, midi) = (app.as_weak(), midi.clone());
        move || {
            let (Some(app), ports) = (weak.upgrade(), midi.borrow().names()) else { return };
            app.set_show_kbd(ports.is_empty());
            app.set_midi_connected(!ports.is_empty());
            app.set_device(if ports.is_empty() { "No MIDI keyboard found. Click the keys or use the letters shown.".into() } else { ports.join(", ").into() });
        }
    };
    midi.borrow_mut().refresh();
    show_devices();
    let hotplug = Timer::default();
    hotplug.start(TimerMode::Repeated, Duration::from_secs(2), move || {
        if midi.borrow_mut().refresh() {
            show_devices();
        }
    });

    app.run()?;
    ctl.borrow().save();
    Ok(())
}

fn strings(items: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.iter().map(|s| SharedString::from(*s)).collect::<Vec<_>>()))
}

fn load_lessons(dir: &str) -> Result<Vec<Lesson>, Box<dyn Error>> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("cannot read lessons in {dir}: {e}"))?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ron"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| Lesson::parse(&fs::read_to_string(p)?).map_err(|e| format!("{}: {e}", p.display()).into()))
        .collect()
}

fn build_keys() -> Vec<KeyState> {
    let mut white = 0.0;
    (LOW..=HIGH)
        .map(|note| {
            let black = is_black(note);
            let x = white; // black keys are centred on the boundary before the next white key
            if !black {
                white += 1.0;
            }
            let label = if note % 12 == 0 { note_name(note) } else { String::new() };
            let kbd = QWERTY.chars().nth(note.wrapping_sub(60) as usize).map(String::from).unwrap_or_default();
            KeyState { note: note as i32, black, x, state: 0, hint: false, found: false, region: false, label: label.into(), kbd: kbd.to_uppercase().into() }
        })
        .collect()
}

/// One timed attempt: a rhythm to tap, or music to play in time.
struct Run {
    run: RhythmRun,
    /// For music: the keys of each moment, so pitch is checked too. Empty for a tapped rhythm.
    keys: Vec<Vec<u8>>,
    /// Wrong keys played.
    wrong: usize,
    /// For music: each moment's start and shortest note length (ms), to check notes are held.
    holds: Vec<(f64, f64)>,
    /// Keys graded and still held down, with their moment.
    held: Vec<(u8, usize)>,
    /// Moments whose note was let go too early.
    short: Vec<bool>,
    /// Where each beat sits on the staff, for the playhead.
    time: Layout,
    beat_ms: f64,
    /// The count-in: time from the first click to the first note.
    lead_ms: f64,
    /// Still being played; false once graded (the marks stay on screen).
    running: bool,
}

/// A round of practice in progress.
enum Drill {
    Reading {
        pool: Vec<NoteCard>,
        card: usize,
        asked_at: f64,
        /// The current note has had a wrong answer.
        missed: bool,
        /// The current note was answered and the next is on its way.
        answered: bool,
        count: usize,
        correct: usize,
        times: Vec<f64>,
    },
    Ear {
        keys: [u8; 2],
        answer: usize,
        missed: bool,
        answered: bool,
        count: usize,
        correct: usize,
    },
}

/// App state and the glue between the lesson engine and the UI.
struct Ctl {
    app: slint::Weak<App>,
    audio: Audio,
    timer: Timer,
    lessons: Vec<Lesson>,
    curriculum: Curriculum,
    /// Pictures by path, loaded on first use.
    images: RefCell<HashMap<String, Image>>,
    save: Save,
    save_path: PathBuf,
    /// Shell page: 0 today, 1 road, 2 library, 3 setup, 4 one stage, 5 practice.
    page: i32,
    /// Which stage of the road page 4 shows.
    stage_open: usize,
    /// The goal picker is open to change an existing goal.
    choosing_goal: bool,
    /// The open lesson's index in `lessons` and its state.
    session: Option<(usize, Session)>,
    /// Per quiz choice: 0 untouched, 1 correct, 2 wrong.
    choice_states: Vec<i32>,
    /// Quiz choices in the order shown: indices into the step's choices, shuffled per visit.
    choice_order: Vec<usize>,
    /// Message under the exercise, and whether it is good news.
    feedback: (String, bool),
    rhythm: Option<Run>,
    /// The calibration screen is open: tap times so far, measured from the first click.
    calibration: Option<Vec<f64>>,
    drill: Option<Drill>,
    /// Mirrors `save.echo_midi` for the MIDI thread.
    echo: Arc<AtomicBool>,
    /// The practice metronome is clicking.
    metronome_on: bool,
    /// Play steps are played in time with a count-in, instead of at the learner's own pace.
    in_time: bool,
    /// Events of the current passage sounding during Listen, and which playback is current.
    listening: Option<Vec<usize>>,
    listen_generation: i32,
    /// The lesson step last shown, to fade in only real step changes.
    shown_step: std::cell::Cell<Option<(usize, usize)>>,
    rng: Rng,
    last_key_ms: Option<f64>,
}

impl Ctl {
    // ---- The road: the chosen goal's stages and their lessons, in order.

    fn goal(&self) -> Option<&Goal> {
        self.save.goal.as_deref().and_then(|id| self.curriculum.goal(id))
    }

    fn road(&self) -> Vec<&Stage> {
        self.goal().map(|g| g.stages.iter().filter_map(|id| self.curriculum.stage(id)).collect()).unwrap_or_default()
    }

    fn lesson_index(&self, id: &str) -> Option<usize> {
        self.lessons.iter().position(|l| l.id == id)
    }

    /// Indices into `lessons` of every lesson along the road.
    fn path(&self) -> Vec<usize> {
        self.road().iter().flat_map(|s| &s.lessons).filter_map(|id| self.lesson_index(id)).collect()
    }

    fn is_done(&self, lesson: usize) -> bool {
        self.save.done.contains(&self.lessons[lesson].id)
    }



    /// (finished, total) over a stage's lessons.
    fn stage_progress(&self, stage: &Stage) -> (usize, usize) {
        (stage.lessons.iter().filter(|id| self.save.done.contains(*id)).count(), stage.lessons.len())
    }

    /// The first stage on the road that is not finished (stages still being written never are).
    fn current_stage(&self) -> Option<usize> {
        self.road().iter().position(|s| {
            let (done, total) = self.stage_progress(s);
            total == 0 || done < total
        })
    }

    /// The first lesson on the road not yet finished.
    fn next_lesson(&self) -> Option<usize> {
        self.path().into_iter().find(|&i| !self.is_done(i))
    }

    fn streak(&self) -> u64 {
        let today = today();
        // A streak survives until the end of the day after the last practice.
        let mut day = if self.save.days.contains(&today) { today } else { today.saturating_sub(1) };
        let mut streak = 0;
        while self.save.days.contains(&day) {
            streak += 1;
            day -= 1;
        }
        streak
    }

    /// Notes a kind of practice as done today.
    fn did(&mut self, activity: &str) {
        self.save.activity.insert(activity.into(), today());
        self.save.days.insert(today());
        self.save();
    }

    fn done_today(&self, activity: &str) -> bool {
        self.save.activity.get(activity) == Some(&today())
    }

    // ---- Navigation

    fn navigate(&mut self, page: i32) {
        // -1 is a drill's own timer asking for its next question.
        if page < 0 {
            return match self.drill {
                Some(Drill::Reading { answered: true, count, .. }) if count < READING_ROUND => self.next_reading_card(),
                Some(Drill::Reading { .. }) => self.show(),
                Some(Drill::Ear { answered: true, .. }) => self.next_ear_question(),
                _ => {}
            };
        }
        self.page = page;
        self.show();
    }



    fn pick_goal(&mut self, index: usize) {
        if let Some(goal) = self.curriculum.goals.get(index) {
            self.save.goal = Some(goal.id.clone());
            self.choosing_goal = false;
            self.page = 0;
            self.save();
            self.show();
        }
    }

    fn change_goal(&mut self) {
        self.choosing_goal = true;
        self.show();
    }

    fn open_stage(&mut self, index: usize) {
        self.stage_open = index;
        self.page = 4;
        self.show();
    }

    /// A dot on the Today page: an index within the current part.
    fn open_part_stage(&mut self, index: usize) {
        let offset = self.part_range().0;
        self.open_stage(offset + index);
    }

    /// Road indices [from, to) of the part the current stage belongs to.
    fn part_range(&self) -> (usize, usize) {
        let road = self.road();
        let Some(i) = self.current_stage().or(road.len().checked_sub(1)) else { return (0, 0) };
        let part = &road[i].part;
        let from = road[..i].iter().rposition(|s| &s.part != part).map_or(0, |p| p + 1);
        let to = road[i..].iter().position(|s| &s.part != part).map_or(road.len(), |p| i + p);
        (from, to)
    }

    fn open_stage_lesson(&mut self, index: usize) {
        let lesson = self.road().get(self.stage_open).and_then(|s| s.lessons.get(index)).and_then(|id| self.lesson_index(id));
        if let Some(lesson) = lesson {
            self.open(lesson);
        }
    }












    fn plan_item(&mut self, kind: i32) {
        match kind {
            0 => self.start_reading(),
            3 => self.start_ear(),
            4 => self.start_sight(),
            _ => self.continue_learning(),
        }
    }

    /// Where the learner left off: a lesson index and step, if that lesson still exists.
    fn resume_point(&self) -> Option<(usize, usize)> {
        let (id, step) = self.save.resume.as_ref()?;
        let lesson = self.lesson_index(id)?;
        Some((lesson, (*step).min(self.lessons[lesson].steps.len() - 1)))
    }

    /// Continue: the exact step last visited, else the next lesson on the road.
    fn continue_learning(&mut self) {
        if let Some((lesson, step)) = self.resume_point() {
            self.open(lesson);
            if let Some((_, session)) = &mut self.session {
                session.jump(step);
            }
            self.enter_step();
        } else if let Some(lesson) = self.next_lesson() {
            self.open(lesson);
        }
    }

    fn open(&mut self, lesson: usize) {
        self.calibration = None;
        self.drill = None;
        self.session = Some((lesson, Session::new(self.lessons[lesson].clone())));
        self.enter_step();
    }

    /// Escape / leave: back to the shell from a lesson, drill, calibration, or a goal change.
    fn home(&mut self) {
        self.calibration = None;
        self.session = None;
        self.drill = None;
        self.choosing_goal = false;
        self.enter_step();
    }

    /// Opens (or restarts) the timing calibration: a plain click to tap along with.
    fn calibrate(&mut self) {
        self.session = None;
        self.drill = None;
        self.enter_step();
        self.calibration = Some(Vec::new());
        self.audio.click(CALIBRATION_BPM, 0);
        self.show();
    }

    fn set_theme(&mut self, mode: i32) {
        self.save.theme = mode;
        if let Some(app) = self.app.upgrade() {
            app.global::<Theme>().set_mode(mode);
        }
        self.save();
    }

    fn toggle_piano(&mut self) {
        self.save.show_keyboard = !self.save.show_keyboard;
        self.save();
        self.show();
    }

    fn toggle_echo(&mut self) {
        self.save.echo_midi = !self.save.echo_midi;
        self.echo.store(self.save.echo_midi, Ordering::Relaxed);
        self.save();
        self.show();
    }

    fn toggle_names(&mut self) {
        self.save.names = !self.save.names;
        self.save();
        self.show();
    }

    /// Enter / the main button: whatever "go on" means on the current screen.
    fn next(&mut self) {
        if self.calibration.is_some() {
            return self.home();
        }
        if self.drill.is_some() {
            return;
        }
        let timed = self.timed_step();
        let Some((index, session)) = &mut self.session else {
            // Today page: the next thing on the road.
            if let (0, false, Some(_)) = (self.page, self.choosing_goal, self.goal()) {
                self.continue_learning();
            }
            return;
        };
        if session.current().is_none() {
            // Lesson complete: on to the one after it on the road, or back to the shell.
            let index = *index;
            let path = self.path();
            let following = path.iter().position(|&i| i == index).and_then(|at| path.get(at + 1)).copied();
            return match following {
                Some(lesson) => self.open(lesson),
                None => self.home(),
            };
        }
        if timed && !session.solved {
            if !self.rhythm.as_ref().is_some_and(|r| r.running) {
                self.start_rhythm();
            }
            return;
        }
        if !session.can_advance() {
            return;
        }
        if session.advance() {
            let id = session.lesson.id.clone();
            // Generated practice rounds count as practice, not as lessons on the road.
            match id.strip_prefix("practice-") {
                Some(kind) => {
                    let kind = kind.to_string();
                    *self.save.rounds.entry(kind.clone()).or_default() += 1;
                    self.did(&kind);
                }
                None => {
                    self.save.done.insert(id);
                    self.did("lesson");
                }
            }
        }
        self.enter_step();
    }

    /// Start the current step again, or the whole lesson from the "complete" screen.
    fn retry(&mut self) {
        let Some((index, session)) = &mut self.session else { return };
        if session.current().is_none() {
            let index = *index;
            return self.open(index);
        }
        session.restart();
        self.enter_step();
    }

    /// Back one step in the lesson.
    fn previous(&mut self) {
        if let Some((_, session)) = &mut self.session {
            session.back();
            self.enter_step();
        }
    }

    fn choose(&mut self, choice: usize) {
        if let Some(Drill::Ear { .. }) = self.drill {
            return self.answer_ear(choice);
        }
        let Some((_, session)) = &mut self.session else { return };
        if self.choice_states.get(choice) != Some(&0) {
            return;
        }
        if let Some(correct) = session.answer(self.choice_order[choice]) {
            self.choice_states[choice] = if correct { 1 } else { 2 };
            self.feedback = if correct { ("That's right.".into(), true) } else { ("Not quite. Have another look.".into(), false) };
            self.update();
        }
    }

    // ---- Keys

    /// A piano key went down or up at time `at`, from any source.
    fn played(&mut self, note: u8, down: bool, at: f64) {
        // Key colour: 0 up, 1 down, 2 down and right, 3 down and wrong.
        let mut state = 0;
        // A passage played by ear was just completed: show its notes.
        let mut reveal = false;
        if down {
            state = 1;
            self.stop_listening();
            self.count_practice(at);
            let latency = self.save.latency_ms.unwrap_or(0.0);
            let click_start = self.audio.click_start();
            if let Some(Drill::Reading { .. }) = self.drill {
                state = self.reading_key(note, at);
            } else if let Some(taps) = &mut self.calibration {
                if let (Some(start), true) = (click_start, taps.len() < CALIBRATION_TAPS) {
                    taps.push(at - start);
                    if taps.len() == CALIBRATION_TAPS {
                        self.save.latency_ms = Some(median_offset(taps, 60_000.0 / CALIBRATION_BPM as f64));
                        self.save();
                        self.audio.sender()(Msg::ClickOff);
                    }
                    self.show();
                }
            } else if let (Some(r), Some(start)) = (self.rhythm.as_mut().filter(|r| r.running), click_start) {
                let t = at - latency - start - r.lead_ms;
                let mark = if r.keys.is_empty() {
                    r.run.tap(t)
                } else if let Some(i) = r.run.nearest(t, |i| r.keys[i].contains(&note)) {
                    r.held.push((note, i));
                    Some(r.run.mark(i, t))
                } else if (0..r.keys.len()).any(|i| r.keys[i].contains(&note) && r.run.near(i, t)) {
                    None // another key of a chord already graded
                } else {
                    if t > -r.run.window_ms() {
                        r.wrong += 1;
                        self.feedback = (format!("Wrong note: {}", note_name(note)), false);
                        state = 3;
                    }
                    None
                };
                if state != 3 {
                    (state, self.feedback) = match mark {
                        Some(Mark::Good) => (2, ("In time".into(), true)),
                        Some(Mark::Early) => (3, ("Early".into(), false)),
                        Some(Mark::Late) => (3, ("Late".into(), false)),
                        _ => (if r.keys.is_empty() { 1 } else { 2 }, std::mem::take(&mut self.feedback)),
                    };
                }
            } else if self.in_time && matches!(self.session.as_ref().and_then(|(_, s)| s.current()), Some(Step::Play { .. })) {
                // Waiting for Start: keys are free to play.
            } else if let Some((_, session)) = &mut self.session {
                let target = session.target();
                match session.note_on(note) {
                    Some(true) => {
                        state = 2;
                        if session.solved && matches!(session.current(), Some(Step::Play { hide: true, .. })) {
                            reveal = true;
                        }
                        let finding = matches!(session.current(), Some(Step::Find { .. }));
                        let text = match (session.solved, finding) {
                            (true, true) => "Found them all.".into(),
                            (true, false) => "Well played.".into(),
                            (false, true) => format!("{} found, {} to go.", note_name(note), session.target().len()),
                            (false, false) => String::new(),
                        };
                        self.feedback = (text, true);
                    }
                    Some(false) => {
                        state = 3;
                        let finding = matches!(session.current(), Some(Step::Find { .. }));
                        // The right letter in the wrong place: say which way to move.
                        let octave = target.iter().filter(|&&t| t % 12 == note % 12).min_by_key(|&&t| t.abs_diff(note)).map(|&t| t as i32 - note as i32);
                        self.feedback = (
                            match octave {
                                Some(d) if d.abs() == 12 => format!("Right note, wrong octave. Go one octave {}.", if d > 0 { "up" } else { "down" }),
                                Some(d) => format!("Right note, wrong octave. Go {} octaves {}.", d.abs() / 12, if d > 0 { "up" } else { "down" }),
                                None => {
                                    let hint = match target.as_slice() {
                                        _ if finding && self.save.show_keyboard => "Look at the shaded part of the keyboard.",
                                        _ if finding && matches!(session.current(), Some(Step::Find { show: Show::Keys(_), .. })) => "Check the diagram again.",
                                        _ if finding => "Keep looking.",
                                        [t] if note < *t => "Try a little higher.",
                                        [t] if note > *t => "Try a little lower.",
                                        _ => "Look at the notes again.",
                                    };
                                    format!("That was {}. {hint}", note_name(note))
                                }
                            },
                            false,
                        );
                    }
                    None => {}
                }
            }
        }
        if !down {
            self.released(note, at);
        }
        if let Some(app) = self.app.upgrade() {
            set_key(&app, note, |k| k.state = state);
        }
        if reveal {
            self.show();
        } else if state > 1 {
            self.update();
        }
    }

    /// A key let go during an In-time run: a half note or longer must be held for most of its length
    /// (shorter notes are left alone: staccato and ordinary detached playing are fine).
    fn released(&mut self, note: u8, at: f64) {
        let (Some(start), latency) = (self.audio.click_start(), self.save.latency_ms.unwrap_or(0.0)) else { return };
        let Some(r) = self.rhythm.as_mut().filter(|r| r.running) else { return };
        let Some(pos) = r.held.iter().position(|h| h.0 == note) else { return };
        let (_, i) = r.held.remove(pos);
        let (onset, length) = r.holds[i];
        let held = at - latency - start - r.lead_ms - onset;
        if length >= r.beat_ms * 1.99 && held < length * 0.6 {
            r.short[i] = true;
            let beats = (length / r.beat_ms).round();
            self.feedback = (format!("Hold it longer: that note lasts {beats} {}.", if beats == 1.0 { "beat" } else { "beats" }), false);
            self.update();
        }
    }

    /// Adds the time since the previous key press to today's practice, unless it was a long pause.
    fn count_practice(&mut self, at: f64) {
        if let Some(gap) = self.last_key_ms.map(|last| at - last).filter(|gap| (0.0..PRACTICE_GAP_MS).contains(gap)) {
            let seconds = self.save.practice.entry(today()).or_default();
            let before = *seconds / 60;
            *seconds += (gap / 1000.0).round() as u32;
            if *seconds / 60 != before {
                self.save(); // once a minute is plenty
            }
        }
        self.last_key_ms = Some(at);
    }

    // ---- Rhythm exercises

    /// A step played against the metronome: a rhythm, or music in "in time" mode.
    fn timed_step(&self) -> bool {
        match self.session.as_ref().and_then(|(_, s)| s.current()) {
            Some(Step::PlayRhythm { .. }) => true,
            Some(Step::Play { .. }) => self.in_time,
            _ => false,
        }
    }

    fn set_mode(&mut self, mode: i32) {
        self.stop_listening();
        self.in_time = mode == 1;
        self.rhythm = None;
        self.timer.stop();
        self.audio.sender()(Msg::ClickOff);
        self.feedback = (String::new(), true);
        if let Some(app) = self.app.upgrade() {
            app.set_playhead(-1.0);
        }
        self.update();
    }

    fn set_tempo(&mut self, percent: i32) {
        self.save.slowdown = 100 - percent.clamp(40, 100);
        self.save();
        self.update();
    }

    /// Starts a timed run: one bar of clicks to count in, then the pattern.
    fn start_rhythm(&mut self) {
        self.stop_listening();
        let Some((_, session)) = &self.session else { return };
        let tempo = self.save.tempo();
        let (run, keys, time, bpm, bar, holds) = match session.current() {
            Some(Step::PlayRhythm { bpm, beats_per_bar, beats, tolerance_ms, .. }) => {
                let time = notation::layout_rhythm(*beats_per_bar, beats);
                (RhythmRun::new(beats, bpm * tempo, *tolerance_ms), Vec::new(), time, bpm * tempo, *beats_per_bar, Vec::new())
            }
            Some(step @ Step::Play { bpm, beats_per_bar, .. }) => {
                let music = step.music().unwrap();
                let beat_ms = 60_000.0 / (bpm * tempo) as f64;
                let onsets = session.moments().iter().map(|m| music.events[m[0]].at as f64 * beat_ms).collect();
                let end = music.events.iter().map(|e| e.at + e.beats).fold(0.0, f32::max) as f64 * beat_ms;
                let time = notation::layout(&music, *beats_per_bar, false);
                let bar = if *beats_per_bar > 0 { *beats_per_bar } else { 4 };
                let holds = session
                    .moments()
                    .iter()
                    .map(|m| {
                        let shortest = m.iter().map(|&e| music.events[e].beats).fold(f32::MAX, f32::min);
                        (music.events[m[0]].at as f64 * beat_ms, shortest as f64 * beat_ms)
                    })
                    .collect();
                (RhythmRun::with_onsets(onsets, end, 150.0), session.targets().to_vec(), time, bpm * tempo, bar, holds)
            }
            _ => return,
        };
        let beat_ms = 60_000.0 / bpm as f64;
        let short = vec![false; holds.len()];
        self.rhythm = Some(Run { run, keys, wrong: 0, holds, held: Vec::new(), short, time, beat_ms, lead_ms: beat_ms * bar as f64, running: true });
        self.audio.click(bpm, bar as u32);
        self.feedback = ("Listen for the count-in".into(), true);
        self.timer.restart();
        self.update();
    }

    /// Every frame of a rhythm run: move the playhead, count in, and finish when the pattern is over.
    fn tick(&mut self) {
        let (Some(app), Some(start)) = (self.app.upgrade(), self.audio.click_start()) else { return };
        let latency = self.save.latency_ms.unwrap_or(0.0);
        let Some(r) = self.rhythm.as_mut().filter(|r| r.running) else { return };
        let t = now_ms() - start - r.lead_ms;
        app.set_playhead(if t < 0.0 { -1.0 } else { r.time.x_at((t / r.beat_ms) as f32) });

        let before = (r.run.marks.clone(), self.feedback.0.clone());
        if t < 0.0 {
            self.feedback = (format!("Count in: {}", ((t + r.lead_ms) / r.beat_ms) as i32 + 1), true);
        } else if self.feedback.0.starts_with("Count in") {
            self.feedback.0.clear();
        }
        r.run.expire(t - latency);
        if t - latency.max(0.0) > r.run.end_ms() + r.run.window_ms() {
            return self.finish_rhythm();
        }
        if before != (r.run.marks.clone(), self.feedback.0.clone()) {
            self.update();
        }
    }

    fn finish_rhythm(&mut self) {
        let (Some(r), Some((_, session))) = (&mut self.rhythm, &mut self.session) else { return };
        r.running = false;
        r.run.expire(f64::INFINITY);
        self.audio.sender()(Msg::ClickOff);
        self.timer.stop();

        let count = |mark| r.run.marks.iter().filter(|&&m| m == mark).count();
        let (good, early, late, missed) = (count(Mark::Good), count(Mark::Early), count(Mark::Late), count(Mark::Missed));
        let music = !r.keys.is_empty();
        let shorts = r.short.iter().filter(|&&s| s).count();
        let passed = r.run.passed() && r.wrong == 0 && shorts == 0;
        session.solved |= passed;
        if passed && !session.lesson.id.starts_with("practice-") {
            let tempo = 100 - self.save.slowdown;
            let best = self.save.in_time.entry(format!("{}:{}", session.lesson.id, session.step)).or_default();
            *best = (*best).max(tempo);
        }
        self.feedback = if passed {
            ("In time, every note. Well played.".into(), true)
        } else {
            let advice = if good == 0 && early + late == 0 {
                if music { "Play each note as the playhead reaches it." } else { "Tap a key on each note after the count-in." }
            } else if r.wrong > 0 {
                if r.wrong == 1 { "One wrong note. Try a slower tempo." } else { "Some wrong notes. Try a slower tempo." }
            } else if shorts > 0 {
                "Some long notes were let go too early: hold each one for its full length."
            } else if early > late {
                "You are rushing: wait for the click."
            } else if late > early {
                "You are dragging: stay with the click."
            } else if missed > 0 {
                "Some notes went by untapped."
            } else if r.run.extra > 0 {
                "A few taps too many: one tap per note."
            } else {
                "Close. Once more."
            };
            (format!("{good} of {} in time. {advice}", r.run.marks.len()), false)
        };
        if let Some(app) = self.app.upgrade() {
            app.set_playhead(-1.0);
        }
        self.update();
    }

    // ---- Note reading

    fn pick_reading_level(&mut self, level: usize) {
        self.save.reading_level = level.min(READING_LEVELS.len() - 1);
        self.save();
        self.show();
    }

    fn start_reading(&mut self) {
        let pool = drill::note_pool(self.save.reading_level);
        let card = drill::pick(&pool, &self.save.cards, None, &mut self.rng);
        self.session = None;
        self.drill = Some(Drill::Reading { pool, card, asked_at: now_ms(), missed: false, answered: false, count: 0, correct: 0, times: Vec::new() });
        self.feedback = (String::new(), true);
        self.show();
    }

    /// A key during note reading; returns the key colour.
    fn reading_key(&mut self, note: u8, at: f64) -> i32 {
        let Some(Drill::Reading { pool, card, asked_at, missed, answered, count, correct, times }) = &mut self.drill else { return 1 };
        if *answered || *count >= READING_ROUND {
            return 1;
        }
        let c = pool[*card];
        let stats = self.save.cards.entry(c.key()).or_default();
        if note != c.pitch.midi() {
            if !*missed {
                stats.record(false, 0.0);
                *missed = true;
            }
            self.feedback = (format!("That was {}. Look again.", note_name(note)), false);
            self.update();
            return 3;
        }
        let ms = at - *asked_at;
        if !*missed {
            stats.record(true, ms as f32);
            *correct += 1;
        }
        times.push(ms);
        *count += 1;
        *answered = true;
        self.feedback = (format!("{} · {:.1} s", note_name(note), ms / 1000.0), true);
        let finished = *count == READING_ROUND;
        if finished {
            self.did("reading");
        }
        self.update();
        // Show the green note for a moment, then the next one.
        let weak = self.app.clone();
        Timer::single_shot(Duration::from_millis(if finished { 500 } else { 350 }), move || {
            if let Some(app) = weak.upgrade() {
                app.invoke_navigate(-1); // -1: advance the running drill
            }
        });
        2
    }

    fn next_reading_card(&mut self) {
        let Some(Drill::Reading { pool, card, asked_at, missed, answered, .. }) = &mut self.drill else { return };
        *card = drill::pick(pool, &self.save.cards, Some(*card), &mut self.rng);
        (*asked_at, *missed, *answered) = (now_ms(), false, false);
        self.feedback = (String::new(), true);
        self.show();
    }

    // ---- Generated practice: sight-reading and rhythm reading

    fn pick_sight_level(&mut self, level: usize) {
        self.save.sight_level = level.min(SIGHT_LEVELS.len() - 1);
        self.save();
        self.show();
    }

    fn pick_rhythm_level(&mut self, level: usize) {
        self.save.rhythm_level = level.min(RHYTHM_LEVELS.len() - 1);
        self.save();
        self.show();
    }

    // ---- Practice metronome

    fn metronome(&self) -> (i32, i32) {
        (if self.save.metronome_bpm == 0 { 80 } else { self.save.metronome_bpm }, if self.save.metronome_beats == 0 { 4 } else { self.save.metronome_beats })
    }

    fn metronome_toggle(&mut self) {
        self.metronome_on = !self.metronome_on;
        let (bpm, beats) = self.metronome();
        if self.metronome_on {
            self.audio.click(bpm as f32, beats as u32);
        } else {
            self.audio.sender()(Msg::ClickOff);
        }
        self.show();
    }

    fn metronome_bpm_by(&mut self, delta: i32) {
        self.save.metronome_bpm = (self.metronome().0 + delta).clamp(40, 200);
        self.metronome_restart();
    }

    fn metronome_beats_set(&mut self, beats: i32) {
        self.save.metronome_beats = beats.clamp(2, 6);
        self.metronome_restart();
    }

    /// Saves a new tempo or metre and, if clicking, restarts with it.
    fn metronome_restart(&mut self) {
        self.save();
        if self.metronome_on {
            let (bpm, beats) = self.metronome();
            self.audio.click(bpm as f32, beats as u32);
        }
        self.show();
    }

    // ---- Mastery

    /// Stars for a lesson: 1 for finishing it, 2 when every timed passage was passed In time at
    /// 80% tempo or more, 3 at full tempo.
    fn stars(&self, lesson: usize) -> i32 {
        if !self.is_done(lesson) {
            return 0;
        }
        let l = &self.lessons[lesson];
        let worst = l.timed_steps().iter().map(|k| self.save.in_time.get(&format!("{}:{k}", l.id)).copied().unwrap_or(0)).min().unwrap_or(100);
        match worst {
            100.. => 3,
            80.. => 2,
            _ => 1,
        }
    }

    fn pick_chord_level(&mut self, level: usize) {
        self.save.chord_level = level.min(CHORD_LEVELS.len() - 1);
        self.save();
        self.show();
    }

    fn start_chords(&mut self) {
        let lesson = drill::chord_finding(self.save.chord_level, &mut self.rng);
        self.open_generated(lesson);
    }

    fn start_sight(&mut self) {
        let lesson = drill::sight_reading(self.save.sight_level, &mut self.rng);
        self.open_generated(lesson);
    }

    fn start_rhythm_reading(&mut self) {
        let lesson = drill::rhythm_reading(self.save.rhythm_level, &mut self.rng);
        self.open_generated(lesson);
    }

    /// Opens a generated lesson, replacing the previous round of the same kind.
    fn open_generated(&mut self, lesson: Lesson) {
        let index = match self.lesson_index(&lesson.id) {
            Some(i) => {
                self.lessons[i] = lesson;
                i
            }
            None => {
                self.lessons.push(lesson);
                self.lessons.len() - 1
            }
        };
        self.open(index);
    }

    // ---- Ear training

    fn pick_ear_level(&mut self, level: usize) {
        self.save.ear_level = level.min(EAR_LEVELS.len() - 1);
        self.save();
        self.show();
    }

    fn start_ear(&mut self) {
        self.session = None;
        let (keys, answer) = drill::interval_question(self.save.ear_level, &mut self.rng);
        self.drill = Some(Drill::Ear { keys, answer, missed: false, answered: false, count: 0, correct: 0 });
        self.choice_states = vec![0; self.ear_choices()];
        self.feedback = (String::new(), true);
        self.show();
        self.replay();
    }

    fn ear_choices(&self) -> usize {
        [2, 4, 7][self.save.ear_level.min(2)]
    }

    /// Plays the current interval: one note, then the other. In a lesson, plays the passage.
    fn replay(&mut self) {
        if self.session.is_some() {
            return self.listen();
        }
        let Some(Drill::Ear { keys: [a, b], count, .. }) = self.drill else { return };
        if count >= EAR_ROUND {
            return;
        }
        let send = self.audio.sender();
        send(Msg::Midi([0x90, a, 90]));
        let s = send.clone();
        Timer::single_shot(Duration::from_millis(700), move || {
            s(Msg::Midi([0x80, a, 0]));
            s(Msg::Midi([0x90, b, 90]));
        });
        Timer::single_shot(Duration::from_millis(1700), move || send(Msg::Midi([0x80, b, 0])));
    }

    fn answer_ear(&mut self, choice: usize) {
        let level = self.save.ear_level;
        let Some(Drill::Ear { answer, missed, answered, count, correct, .. }) = &mut self.drill else { return };
        if *answered || choice >= self.choice_states.len() || self.choice_states[choice] != 0 {
            return;
        }
        let right = choice == *answer;
        self.choice_states[choice] = if right { 1 } else { 2 };
        if !right {
            *missed = true;
            self.feedback = ("Not that one. Listen again and try another.".into(), false);
            self.update();
            return;
        }
        let tally = self.save.ear.entry(level).or_default();
        tally.1 += 1;
        if !*missed {
            *correct += 1;
            tally.0 += 1;
        }
        *count += 1;
        *answered = true;
        self.feedback = (format!("Yes: a {}.", INTERVALS[*answer]), true);
        let finished = *count == EAR_ROUND;
        if finished {
            self.did("ear");
        }
        self.update();
        let weak = self.app.clone();
        Timer::single_shot(Duration::from_millis(1100), move || {
            if let Some(app) = weak.upgrade() {
                app.invoke_navigate(-1);
            }
        });
    }

    fn next_ear_question(&mut self) {
        let level = self.save.ear_level;
        let Some(Drill::Ear { keys, answer, missed, answered, count, .. }) = &mut self.drill else { return };
        if *count >= EAR_ROUND {
            return self.show();
        }
        (*keys, *answer) = drill::interval_question(level, &mut self.rng);
        (*missed, *answered) = (false, false);
        self.choice_states = vec![0; self.ear_choices()];
        self.feedback = (String::new(), true);
        self.show();
        self.replay();
    }

    // ---- Listen: hear the passage before playing it

    fn listen(&mut self) {
        let Some(step @ Step::Play { bpm, .. }) = self.session.as_ref().and_then(|(_, s)| s.current()) else { return };
        let music = step.music().unwrap();
        let beat_ms = 60_000.0 / (*bpm * self.save.tempo()) as f64;
        self.stop_listening();
        self.listen_generation += 1;
        self.listening = Some(Vec::new());
        let generation = self.listen_generation;
        let mut end = 0.0;
        for (i, e) in music.events.iter().enumerate().filter(|(_, e)| !e.pitches.is_empty()) {
            let (on, off) = (e.at as f64 * beat_ms, (e.at + e.beats) as f64 * beat_ms - 40.0);
            end = f64::max(end, off);
            for (at, down) in [(on, true), (off, false)] {
                let weak = self.app.clone();
                Timer::single_shot(Duration::from_millis(at as u64 + 150), move || {
                    if let Some(app) = weak.upgrade() {
                        app.invoke_listen_tick(generation, i as i32, down);
                    }
                });
            }
        }
        let weak = self.app.clone();
        Timer::single_shot(Duration::from_millis(end as u64 + 300), move || {
            if let Some(app) = weak.upgrade() {
                app.invoke_listen_tick(generation, -1, false);
            }
        });
        self.update();
    }

    /// One scheduled note of a Listen playback; event -1 ends it. Stale playbacks are ignored.
    fn listen_tick(&mut self, generation: i32, event: i32, on: bool) {
        if generation != self.listen_generation || self.listening.is_none() {
            return;
        }
        if event < 0 {
            return self.stop_listening();
        }
        let Some(music) = self.session.as_ref().and_then(|(_, s)| s.current()).and_then(Step::music) else { return };
        let Some(e) = music.events.get(event as usize) else { return };
        let send = self.audio.sender();
        let app = self.app.upgrade();
        for p in &e.pitches {
            send(Msg::Midi([if on { 0x90 } else { 0x80 }, p.midi(), 80]));
            if let Some(app) = &app {
                set_key(app, p.midi(), |k| k.state = on as i32);
            }
        }
        let sounding = self.listening.get_or_insert_with(Vec::new);
        sounding.retain(|&i| i != event as usize);
        if on {
            sounding.push(event as usize);
        }
        self.update();
    }

    /// Silences a Listen playback and lets go of its keys.
    fn stop_listening(&mut self) {
        let Some(sounding) = self.listening.take() else { return };
        self.listen_generation += 1;
        let music = self.session.as_ref().and_then(|(_, s)| s.current()).and_then(Step::music).unwrap_or_default();
        let app = self.app.upgrade();
        for i in sounding {
            for p in &music.events[i].pitches {
                self.audio.sender()(Msg::Midi([0x80, p.midi(), 0]));
                if let Some(app) = &app {
                    set_key(app, p.midi(), |k| k.state = 0);
                }
            }
        }
        self.update();
    }

    fn save(&self) {
        // Write beside the file, then rename, so a crash mid-write cannot destroy earlier progress.
        let tmp = self.save_path.with_extension("tmp");
        let result = fs::create_dir_all(self.save_path.parent().unwrap())
            .and_then(|_| fs::write(&tmp, ron::to_string(&self.save).unwrap()))
            .and_then(|_| fs::rename(&tmp, &self.save_path));
        if let Err(e) = result {
            eprintln!("could not save progress to {}: {e}", self.save_path.display());
        }
    }

    /// A new step or screen: stop anything timed, reset per-step state and redraw everything.
    fn enter_step(&mut self) {
        self.metronome_on = false; // a lesson or drill uses the click itself
        // Remember the exact place, so Continue returns here even after the app is closed.
        if let Some((_, session)) = self.session.as_ref().filter(|(_, s)| !s.lesson.id.starts_with("practice-")) {
            let here = session.current().map(|_| (session.lesson.id.clone(), session.step));
            if self.save.resume != here {
                self.save.resume = here;
                self.save();
            }
        }
        self.stop_listening();
        self.audio.sender()(Msg::ClickOff);
        self.timer.stop();
        self.rhythm = None;
        self.feedback = match self.session.as_ref().map(|(_, s)| (s.current(), s.target().len())) {
            Some((Some(Step::Find { .. }), n)) => (format!("{n} {} to find.", if n == 1 { "key" } else { "keys" }), true),
            _ => (String::new(), true),
        };
        self.choice_states = match self.session.as_ref().and_then(|(_, s)| s.current()) {
            Some(Step::Quiz { choices, .. }) => vec![0; choices.len()],
            _ => Vec::new(),
        };
        // A fresh order every visit, so the right answer's position teaches nothing.
        self.choice_order = (0..self.choice_states.len()).collect();
        for i in (1..self.choice_order.len()).rev() {
            let j = self.rng.below(i + 1);
            self.choice_order.swap(i, j);
        }
        if let Some(app) = self.app.upgrade() {
            app.set_playhead(-1.0);
        }
        self.show();
    }

    // ---- Pushing state to the UI

    /// A picture by path, decoded once.
    fn picture(&self, path: &str) -> Image {
        self.images
            .borrow_mut()
            .entry(path.to_string())
            .or_insert_with(|| {
                Image::load_from_path(Path::new(path)).unwrap_or_else(|_| {
                    eprintln!("could not load picture {path}");
                    Image::default()
                })
            })
            .clone()
    }

    fn image(&self, name: &str) -> Image {
        self.picture(&format!("{CONTENT}/images/{name}"))
    }

    fn stage_emblem(stage: &Stage) -> SharedString {
        emblem(&stage.emblem).map(String::from).unwrap_or_default().into()
    }

    /// "Author, Title" for each book a stage draws on.
    fn sources(&self, stage: &Stage) -> String {
        let books: Vec<String> = stage.books.iter().filter_map(|id| self.curriculum.book(id)).map(|b| format!("{}, {}", b.author, b.title)).collect();
        books.join("; ")
    }

    fn goal_card(&self, goal: &Goal) -> GoalCard {
        let stages: Vec<&Stage> = goal.stages.iter().filter_map(|id| self.curriculum.stage(id)).collect();
        let lessons: usize = stages.iter().map(|s| s.lessons.len()).sum();
        GoalCard {
            title: goal.title.as_str().into(),
            tagline: goal.tagline.as_str().into(),
            blurb: goal.blurb.as_str().into(),
            meta: format!("{} stages · {lessons} interactive lessons ready", stages.len()).into(),
            image: self.image(&goal.image),
        }
    }

    fn stage_card(&self, i: usize, s: &Stage, current: Option<usize>, prev_part: Option<&str>) -> StageCard {
        let (done, total) = self.stage_progress(s);
        StageCard {
            number: (i + 1).to_string().into(),
            title: s.title.as_str().into(),
            source: self.sources(s).into(),
            blurb: s.blurb.as_str().into(),
            image: self.image(&s.image),
            emblem: Self::stage_emblem(s),
            part: s.part.as_str().into(),
            part_start: prev_part != Some(s.part.as_str()),
            done: done as i32,
            total: total as i32,
            state: if total > 0 && done == total { 2 } else if current == Some(i) { 1 } else { 0 },
        }
    }


    /// Pushes the screen and its static content to the UI.
    /// Screens: 0 shell, 1 lesson, 2 lesson complete, 3 calibration, 4 goal picker, 5 reading, 6 ear.
    fn show(&self) {
        let Some(app) = self.app.upgrade() else { return };
        app.set_names(self.save.names);
        app.set_echo_midi(self.save.echo_midi);
        app.set_show_piano(self.save.show_keyboard);
        if let Some(drill) = &self.drill {
            self.show_drill(&app, drill);
        } else if let Some(taps) = &self.calibration {
            app.set_calibration_taps(taps.len() as i32);
            app.set_calibration_result(match self.save.latency_ms {
                Some(ms) if taps.len() == CALIBRATION_TAPS => {
                    format!("Your taps land {}. Rhythm exercises now allow for that.", describe_latency(ms)).into()
                }
                _ => "".into(),
            });
            app.set_screen(3);
        } else if let Some((index, session)) = &self.session {
            let path = self.path();
            app.set_lesson_title(session.lesson.title.as_str().into());
            app.set_lesson_source(session.lesson.source.as_str().into());
            app.set_step(session.step as i32);
            app.set_step_count(session.lesson.steps.len() as i32);
            app.set_has_next(path.iter().position(|i| i == index).is_some_and(|at| at + 1 < path.len()));
            let Some(step) = session.current() else {
                let stars = if session.lesson.id.starts_with("practice-") { 0 } else { self.stars(*index) };
                app.set_lesson_stars(stars);
                app.set_star_hint(
                    match stars {
                        3 => "Mastered: every passage In time at full tempo.",
                        2 => "Next star: play every passage In time at 100% tempo.",
                        1 => "Next star: play every passage In time at 80% tempo or more.",
                        _ => "",
                    }
                    .into(),
                );
                app.set_screen(2);
                self.update();
                return;
            };
            let names = self.save.names;
            let (kind, body, staff) = match step {
                Step::Explain { text, show } => (0, text, staff_of(show, names)),
                Step::Quiz { question, show, .. } => (1, question, staff_of(show, names)),
                Step::Find { prompt, show, .. } => (4, prompt, staff_of(show, names)),
                // Played by ear: the notes stay hidden until the passage is played right.
                Step::Play { prompt, hide: true, .. } if !session.solved => (2, prompt, None),
                Step::Play { prompt, beats_per_bar, names: own, .. } => {
                    (2, prompt, Some(notation::layout(&step.music().unwrap(), *beats_per_bar, *own || names)))
                }
                Step::PlayRhythm { prompt, beats_per_bar, beats, .. } => (3, prompt, Some(notation::layout_rhythm(*beats_per_bar, beats))),
            };
            if self.shown_step.replace(Some((*index, session.step))) != Some((*index, session.step)) {
                app.set_entered(false);
                let weak = self.app.clone();
                Timer::single_shot(Duration::from_millis(30), move || {
                    if let Some(app) = weak.upgrade() {
                        app.set_entered(true);
                    }
                });
            }
            set_staff(&app, staff);
            let listen = Show::Figure("listen".into());
            set_figure(&app, match step {
                Step::Explain { show, .. } | Step::Quiz { show, .. } | Step::Find { show, .. } => Some(show),
                Step::Play { hide: true, .. } if !session.solved => Some(&listen),
                _ => None,
            });
            if let Step::Play { beats_per_bar, bpm, tips, .. } = step {
                set_hand_panel(&app, &step.music().unwrap(), *beats_per_bar, (bpm * self.save.tempo()).round(), tips);
            }
            app.set_kind(kind);
            app.set_body(body.as_str().into());
            app.set_screen(1);
        } else if let (Some(goal), false) = (self.goal(), self.choosing_goal) {
            self.show_shell(&app, goal);
            app.set_screen(0);
        } else {
            let goals: Vec<GoalCard> = self.curriculum.goals.iter().map(|g| self.goal_card(g)).collect();
            app.set_goals(ModelRc::new(VecModel::from(goals)));
            app.set_screen(4);
        }
        self.update();
    }

    fn show_drill(&self, app: &App, drill: &Drill) {
        let mut state = DrillState::default();
        match drill {
            Drill::Reading { pool, card, count, correct, times, .. } => {
                let c = pool[*card];
                state.title = "Note reading".into();
                state.level = READING_LEVELS[self.save.reading_level].into();
                state.prompt = "Play this note".into();
                (state.count, state.total, state.correct) = (*count as i32, READING_ROUND as i32, *correct as i32);
                state.finished = *count >= READING_ROUND;
                if state.finished {
                    let avg = times.iter().sum::<f64>() / times.len().max(1) as f64 / 1000.0;
                    state.score = format!("{correct} / {READING_ROUND}").into();
                    state.detail = format!("right first time · {avg:.1} s average per note").into();
                    state.advice = if *correct == READING_ROUND && avg < 1.5 {
                        "Fluent. Try the next level."
                    } else if *correct as f64 >= READING_ROUND as f64 * 0.8 {
                        "Solid. Another round makes the slow notes quicker."
                    } else {
                        "Keep going: the notes you missed will come up more often."
                    }
                    .into();
                } else {
                    set_staff(app, Some(notation::layout_pitches(c.clef, &[c.pitch])));
                }
                app.set_screen(5);
            }
            Drill::Ear { count, correct, .. } => {
                state.title = "Ear training".into();
                state.level = EAR_LEVELS[self.save.ear_level].into();
                state.prompt = "Which interval did you hear?".into();
                (state.count, state.total, state.correct) = (*count as i32, EAR_ROUND as i32, *correct as i32);
                state.finished = *count >= EAR_ROUND;
                if state.finished {
                    state.score = format!("{correct} / {EAR_ROUND}").into();
                    state.detail = "named right first time".into();
                    state.advice = if *correct >= 9 { "A good ear. Move up a level when it feels easy." } else { "Sing the two notes back before you answer: it helps." }.into();
                }
                app.set_screen(6);
            }
        }
        state.feedback = self.feedback.0.as_str().into();
        state.feedback_good = self.feedback.1;
        app.set_drill(state);
    }

    /// Everything the sidebar pages show.
    fn show_shell(&self, app: &App, goal: &Goal) {
        let road = self.road();
        let path = self.path();
        let current = self.current_stage();
        let lessons_done = path.iter().filter(|&&i| self.is_done(i)).count();
        let (done, total) = road.iter().map(|s| self.stage_progress(s)).fold((0, 0), |(a, b), (c, d)| (a + c, b + d));

        let mut prev_part = None;
        let stages: Vec<StageCard> = road
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let card = self.stage_card(i, s, current, prev_part);
                prev_part = Some(s.part.as_str());
                card
            })
            .collect();

        app.set_page(self.page);
        app.set_goal(self.goal_card(goal));
        app.set_progress(if total == 0 { 0.0 } else { done as f32 / total as f32 });
        app.set_progress_short(format!("{done} of {total} lessons").into());
        app.set_progress_text(format!("{done} of {total} lessons written so far · {} stages on this road", road.len()).into());
        let streak = self.streak();
        app.set_streak(if streak > 0 { format!("{streak}-day streak").into() } else { "".into() });
        app.set_greeting(if self.save.days.is_empty() { "Welcome".into() } else { "Welcome back".into() });
        let mastered = drill::note_pool(3).iter().filter(|c| self.save.cards.get(&c.key()).is_some_and(|s| s.seen >= 3 && (s.missed as f32) < 0.15 * s.seen as f32 && s.ms < 1500.0)).count();
        app.set_stats(ModelRc::new(VecModel::from(vec![
            Stat { value: lessons_done.to_string().into(), label: "Lessons completed".into() },
            Stat { value: mastered.to_string().into(), label: "Notes you read fluently".into() },
            Stat { value: format!("{streak} {}", if streak == 1 { "day" } else { "days" }).into(), label: "Practice streak".into() },
        ])));

        // The current part of the road, as dots.
        let (from, to) = self.part_range();
        app.set_part_stages(ModelRc::new(VecModel::from(stages[from..to].to_vec())));
        app.set_road_title(road.get(from).map_or("Your road".into(), |s| s.part.clone()).into());
        app.set_road_line(match current {
            Some(i) => match road.get(i + 1) {
                Some(next) => format!("Now: {}. Next: {}.", road[i].title, next.title),
                None => format!("Now: {}, the last stage of this road.", road[i].title),
            },
            None => "You have finished every stage of this road.".into(),
        }
        .into());

        let resume = self.resume_point();
        app.set_next_up(match resume.map(|r| r.0).or(self.next_lesson()) {
            Some(lesson) => {
                let l = &self.lessons[lesson];
                let (i, stage) = road.iter().enumerate().find(|(_, s)| s.lessons.contains(&l.id)).unwrap_or((0, &road[0]));
                let summary = if l.source.is_empty() { &l.summary } else { &l.source };
                NextUp {
                    kicker: match resume {
                        Some(_) => format!("Continue where you left off · Stage {} · {}", i + 1, stage.title),
                        None => format!("Up next · Stage {} · {}", i + 1, stage.title),
                    }
                    .into(),
                    title: l.title.as_str().into(),
                    source: match resume {
                        Some((_, step)) => format!("Step {} of {} · {summary}", step + 1, l.steps.len()),
                        None => summary.clone(),
                    }
                    .into(),
                    action: if lessons_done == 0 && resume.is_none() { "Start" } else { "Continue" }.into(),
                    image: self.image(&stage.image),
                    emblem: Self::stage_emblem(stage),
                }
            }
            None => NextUp {
                kicker: "All caught up".into(),
                title: "Everything written for this road is done".into(),
                source: "The next stage is in preparation. Keep your skills warm with the drills in Practice.".into(),
                action: "".into(),
                image: self.image(&goal.image),
                emblem: "".into(),
            },
        });

        // Today's practice plan.
        let next_lesson = resume.map(|r| r.0).or_else(|| path.iter().copied().find(|&i| !self.is_done(i)));
        app.set_plan(ModelRc::new(VecModel::from(vec![
            PlanItem {
                title: "Warm up: note reading".into(),
                detail: format!("{} · 20 notes, about two minutes", READING_LEVELS[self.save.reading_level]).into(),
                done: self.done_today("reading"),
                kind: 0,
            },
            PlanItem {
                title: next_lesson.map_or("Lesson: all done".into(), |l| format!("Lesson: {}", self.lessons[l].title)).into(),
                detail: "Interactive, checked note by note".into(),
                done: self.done_today("lesson"),
                kind: 1,
            },
            PlanItem {
                title: "Sight-reading".into(),
                detail: format!("{} · three new passages", SIGHT_LEVELS[self.save.sight_level]).into(),
                done: self.done_today("sight"),
                kind: 4,
            },
            PlanItem {
                title: "Ear training".into(),
                detail: format!("{} · 10 intervals", EAR_LEVELS[self.save.ear_level]).into(),
                done: self.done_today("ear"),
                kind: 3,
            },
        ])));

        // The last seven days of playing time.
        let today = today();
        let week: Vec<DayBar> = (0..7)
            .rev()
            .map(|back| {
                let day = today - back;
                // Day 0 (1 Jan 1970) was a Thursday.
                let label = ["Th", "Fr", "Sa", "Su", "Mo", "Tu", "We"][(day % 7) as usize];
                DayBar { label: label.into(), minutes: (self.save.practice.get(&day).copied().unwrap_or(0) / 60) as i32, today: back == 0 }
            })
            .collect();
        let total_minutes: i32 = week.iter().map(|d| d.minutes).sum();
        app.set_week_total(format!("{total_minutes} minutes at the keys").into());
        app.set_week(ModelRc::new(VecModel::from(week)));

        // One stage, when open.
        if let (Some(stage), Some(card)) = (road.get(self.stage_open), stages.get(self.stage_open)) {
            app.set_stage(card.clone());
            app.set_stage_kicker(format!("Stage {} of {} · {}", self.stage_open + 1, road.len(), stage.part).into());
            let cards: Vec<Card> = stage
                .lessons
                .iter()
                .filter_map(|id| self.lesson_index(id))
                .enumerate()
                .map(|(n, i)| Card {
                    number: (n + 1).to_string().into(),
                    title: self.lessons[i].title.as_str().into(),
                    summary: self.lessons[i].summary.as_str().into(),
                    done: self.is_done(i),
                    stars: self.stars(i),
                })
                .collect();
            app.set_cards(ModelRc::new(VecModel::from(cards)));
        }
        app.set_stages(ModelRc::new(VecModel::from(stages)));

        // Practice.
        self.show_practice(app);

        // Library.
        let books: Vec<BookCard> = self
            .curriculum
            .books
            .iter()
            .map(|b| {
                let used: Vec<&str> = self.curriculum.stages.iter().filter(|s| s.books.contains(&b.id)).map(|s| s.title.as_str()).collect();
                BookCard {
                    title: b.title.as_str().into(),
                    byline: format!("{} · {}", b.author, b.year).into(),
                    blurb: b.blurb.as_str().into(),
                    used: if used.len() > 4 { format!("Used in {} stages", used.len()) } else { format!("Used in: {}", used.join(", ")) }.into(),
                    image: self.image(&b.image),
                }
            })
            .collect();
        app.set_books(ModelRc::new(VecModel::from(books)));

        app.set_calibration(Card {
            number: "".into(),
            title: "Calibrate timing".into(),
            summary: match self.save.latency_ms {
                Some(ms) => format!("Taps land {}. Redo after changing speakers.", describe_latency(ms)).into(),
                None => "Tap along with a click so rhythm exercises judge you fairly on this device.".into(),
            },
            done: self.save.latency_ms.is_some(),
            stars: 0,
        });
    }

    fn show_practice(&self, app: &App) {
        let pool = drill::note_pool(self.save.reading_level);
        let seen: Vec<(NoteCard, CardStats)> = pool.iter().filter_map(|c| self.save.cards.get(&c.key()).filter(|s| s.seen > 0).map(|s| (*c, *s))).collect();
        let (answers, misses) = seen.iter().fold((0, 0), |(a, m), (_, s)| (a + s.seen, m + s.missed));
        let timed: Vec<f32> = seen.iter().map(|(_, s)| s.ms).filter(|ms| *ms > 0.0).collect();
        let mastered = seen.iter().filter(|(_, s)| s.seen >= 3 && (s.missed as f32) < 0.15 * s.seen as f32 && s.ms < 1500.0).count();
        app.set_reading_level(self.save.reading_level as i32);
        app.set_reading_stats(ModelRc::new(VecModel::from(vec![
            Stat { value: if answers == 0 { "–".into() } else { format!("{}%", 100 * (answers - misses) / answers).into() }, label: "Accuracy".into() },
            Stat {
                value: if timed.is_empty() { "–".into() } else { format!("{:.1} s", timed.iter().sum::<f32>() / timed.len() as f32 / 1000.0).into() },
                label: "Average answer".into(),
            },
            Stat { value: format!("{mastered} / {}", pool.len()).into(), label: "Notes mastered".into() },
        ])));
        let mut weak: Vec<&(NoteCard, CardStats)> = seen.iter().filter(|(_, s)| s.missed > 0 || s.ms > 1500.0).collect();
        weak.sort_by(|a, b| {
            let score = |s: &CardStats| s.missed as f32 / s.seen as f32 + s.ms / 3000.0;
            score(&b.1).total_cmp(&score(&a.1))
        });
        app.set_weak_notes(ModelRc::new(VecModel::from(
            weak.iter()
                .take(6)
                .map(|(c, s)| {
                    let clef = if c.clef == pianomate_core::lesson::Clef::Treble { "treble" } else { "bass" };
                    let speed = if s.ms > 0.0 { format!(" · {:.1} s", s.ms / 1000.0) } else { String::new() };
                    Stat { value: note_name(c.pitch.midi()).into(), label: format!("{clef} · {} of {} right{speed}", s.seen - s.missed, s.seen).into() }
                })
                .collect::<Vec<_>>(),
        )));
        let (right, asked) = self.save.ear.get(&self.save.ear_level).copied().unwrap_or_default();
        app.set_ear_level(self.save.ear_level as i32);
        app.set_ear_stats(ModelRc::new(VecModel::from(vec![
            Stat { value: if asked == 0 { "–".into() } else { format!("{}%", 100 * right / asked).into() }, label: "Right first time".into() },
            Stat { value: asked.to_string().into(), label: "Intervals heard".into() },
        ])));
        app.set_reading_art(self.image("bach.jpg"));
        let (bpm, beats) = self.metronome();
        app.set_metronome_on(self.metronome_on);
        app.set_metronome_bpm(bpm);
        app.set_metronome_beats(beats);
        app.set_sight_art(self.image("renoir.jpg"));
        app.set_rhythm_art(self.image("beethoven.jpg"));
        let rounds = |kind: &str| Stat { value: self.save.rounds.get(kind).copied().unwrap_or(0).to_string().into(), label: "Rounds played".into() };
        let today = |kind: &str| Stat { value: if self.done_today(kind) { "Done" } else { "Not yet" }.into(), label: "Today".into() };
        app.set_sight_level(self.save.sight_level as i32);
        app.set_sight_stats(ModelRc::new(VecModel::from(vec![rounds("sight"), today("sight")])));
        app.set_rhythm_level(self.save.rhythm_level as i32);
        app.set_rhythm_stats(ModelRc::new(VecModel::from(vec![rounds("rhythm"), today("rhythm")])));
        app.set_chord_art(self.image("gachet.jpg"));
        app.set_chord_level(self.save.chord_level as i32);
        app.set_chord_stats(ModelRc::new(VecModel::from(vec![rounds("chords"), today("chords")])));
        app.set_ear_art(self.image("gachet.jpg"));
    }

    /// Pushes what changes within a step: progress, feedback, quiz marks and key markers.
    fn update(&self) {
        let Some(app) = self.app.upgrade() else { return };
        if let Some(drill) = &self.drill {
            let mut state = app.get_drill();
            state.feedback = self.feedback.0.as_str().into();
            state.feedback_good = self.feedback.1;
            match drill {
                Drill::Reading { answered, count, correct, .. } => {
                    app.set_marks(ModelRc::new(VecModel::from(vec![if *answered { 1 } else { 0 }])));
                    (state.count, state.correct) = (*count as i32, *correct as i32);
                }
                Drill::Ear { count, correct, .. } => {
                    let choices: Vec<Choice> = INTERVALS[..self.choice_states.len()]
                        .iter()
                        .zip(&self.choice_states)
                        .map(|(text, &state)| Choice { text: (*text).into(), state })
                        .collect();
                    app.set_choices(ModelRc::new(VecModel::from(choices)));
                    (state.count, state.correct) = (*count as i32, *correct as i32);
                }
            }
            app.set_drill(state);
            for note in LOW..=HIGH {
                set_key(&app, note, |k| (k.hint, k.region) = (false, false));
            }
            return;
        }
        let session = self.session.as_ref().map(|(_, s)| s);
        let step = session.and_then(|s| s.current());
        let running = self.rhythm.as_ref().is_some_and(|r| r.running);
        let rhythm_open = self.timed_step() && !session.is_some_and(|s| s.solved);
        app.set_in_time(self.in_time);
        app.set_tempo(100 - self.save.slowdown);

        app.set_can_continue(if rhythm_open { !running } else { session.is_some_and(|s| s.can_advance()) });
        app.set_action(
            match (rhythm_open, running, self.rhythm.is_some(), session) {
                (true, true, ..) if matches!(step, Some(Step::PlayRhythm { .. })) => "Tap along",
                (true, true, ..) => "Play along",
                (true, false, true, _) => "Try again",
                (true, false, false, _) => "Start",
                (false, _, _, Some(s)) if s.step + 1 == s.lesson.steps.len() => "Finish lesson",
                _ => "Continue",
            }
            .into(),
        );
        app.set_feedback(self.feedback.0.as_str().into());
        app.set_feedback_good(self.feedback.1);

        // Per event on the staff: 0 plain, 1 right, 2 wrong, 3 next to play.
        let marks: Vec<i32> = match (step, session, &self.rhythm) {
            (Some(Step::Play { .. }), Some(s), run) => match (&self.listening, run) {
                (Some(sounding), _) => (0..s.marks().len()).map(|i| if sounding.contains(&i) { 3 } else { 0 }).collect(),
                // Playing in time: colour each moment's events by its timing mark.
                (None, Some(r)) if !r.keys.is_empty() => {
                    let mut marks = vec![0; s.music().events.len()];
                    for (k, (moment, mark)) in s.moments().iter().zip(&r.run.marks).enumerate() {
                        for &e in moment {
                            marks[e] = match mark {
                                _ if r.short[k] => 2,
                                Mark::Pending => 0,
                                Mark::Good => 1,
                                _ => 2,
                            };
                        }
                    }
                    marks
                }
                (None, _) if self.in_time => vec![0; s.music().events.len()],
                (None, _) => s.marks(),
            },
            (Some(Step::PlayRhythm { .. }), _, Some(r)) => r
                .run
                .marks
                .iter()
                .map(|m| match m {
                    Mark::Pending => 0,
                    Mark::Good => 1,
                    _ => 2,
                })
                .collect(),
            _ => Vec::new(),
        };
        app.set_marks(ModelRc::new(VecModel::from(marks)));

        let choices: Vec<Choice> = match step {
            Some(Step::Quiz { choices, .. }) => {
                self.choice_order.iter().zip(&self.choice_states).map(|(&i, &state)| Choice { text: choices[i].as_str().into(), state }).collect()
            }
            _ => Vec::new(),
        };
        app.set_choices(ModelRc::new(VecModel::from(choices)));

        let hints: Vec<u8> = match step {
            Some(Step::Play { hint: true, hide: false, .. }) if !self.in_time => session.map(|s| s.target()).unwrap_or_default(),
            Some(Step::Explain { show: Show::Keyboard(pitches), .. } | Step::Quiz { show: Show::Keyboard(pitches), .. }) => {
                pitches.iter().map(|p| p.midi()).collect()
            }
            _ => Vec::new(),
        };
        let found: &[u8] = match (step, session) {
            (Some(Step::Find { .. }), Some(s)) => s.found(),
            _ => &[],
        };
        // The stretch of keyboard a diagram shows, so it is clear where on the piano to look.
        let region = match step {
            Some(Step::Explain { show: Show::Keys(d), .. } | Step::Quiz { show: Show::Keys(d), .. } | Step::Find { show: Show::Keys(d), .. }) => d.from.midi()..=d.to.midi(),
            // A find step without a diagram: shade the octaves its keys are in.
            Some(Step::Find { keys, .. }) => {
                let (lo, hi) = keys.iter().map(|p| p.midi()).fold((u8::MAX, 0), |(a, b), k| (a.min(k), b.max(k)));
                (lo - lo % 12)..=(hi - hi % 12 + 11)
            }
            _ => 1..=0,
        };
        for note in LOW..=HIGH {
            set_key(&app, note, |k| (k.hint, k.found, k.region) = (hints.contains(&note), found.contains(&note), region.contains(&note)));
        }
        let diagram = app.get_diagram_keys();
        for row in 0..diagram.row_count() {
            let mut key = diagram.row_data(row).unwrap();
            let now = found.contains(&(key.note as u8));
            if key.found != now {
                key.found = now;
                diagram.set_row_data(row, key);
            }
        }
    }
}

/// Fills the panel beside music to play: a small keyboard with each hand's keys and fingers,
/// where each hand sits, what the passage contains, and the lesson's tips.
fn set_hand_panel(app: &App, music: &Music, beats_per_bar: u8, bpm: f32, tips: &[String]) {
    use pianomate_core::music::Hand;
    let hands: Vec<(Hand, Vec<(u8, Option<u8>)>)> =
        [Hand::Right, Hand::Left].into_iter().map(|h| (h, music.fingering(h))).filter(|(_, keys)| !keys.is_empty()).collect();
    let all: Vec<u8> = hands.iter().flat_map(|(_, keys)| keys.iter().map(|k| k.0)).collect();
    let (lo, hi) = (*all.iter().min().unwrap(), *all.iter().max().unwrap());
    // Start and end on white keys, at least a fifth wide, so the hand's surroundings show.
    let mut from = lo - is_black(lo) as u8;
    let mut to = hi + is_black(hi) as u8;
    while to - from < 7 {
        from -= 1 + is_black(from - 1) as u8;
        to += 1 + is_black(to + 1) as u8;
    }
    let pitch = |k: u8| pianomate_core::Pitch { step: (k / 12) as i8 * 7 - 7 + [0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 5, 6][(k % 12) as usize], alter: is_black(k) as i8 };
    let marks = hands.iter().flat_map(|(_, keys)| keys.iter().map(|&(k, f)| (pitch(k), f.map(|f| f.to_string()).unwrap_or_default()))).collect();
    let (mut keys, _, whites, _) = diagram(&KeyDiagram { from: pitch(from), to: pitch(to), marks, arcs: Vec::new() });
    for key in &mut keys {
        if let Some((h, _)) = hands.iter().find(|(_, ks)| ks.iter().any(|k| k.0 as i32 == key.note)) {
            key.hand = if *h == Hand::Left { 2 } else { 1 };
        }
    }
    let lines: Vec<SharedString> = hands
        .iter()
        .filter_map(|(h, _)| Some(format!("{}: {}", if *h == Hand::Left { "Left hand" } else { "Right hand" }, music.position(*h)?).into()))
        .collect();

    // What the passage contains.
    let mut facts = vec![format!(
        "{} · {}",
        match hands.len() {
            2 => "Both hands",
            _ if hands[0].0 == Hand::Left => "Left hand alone",
            _ => "Right hand alone",
        },
        if beats_per_bar > 0 { format!("{beats_per_bar} beats in a bar, ♩ = {bpm}") } else { format!("free rhythm, ♩ = {bpm} for Listen") }
    )];
    let mut letters: Vec<u8> = all.clone();
    letters.sort_unstable();
    let names: Vec<String> = letters.iter().map(|&k| note_name(k)).collect();
    facts.push(format!("Notes: {}", names.join(" ")));
    let black: Vec<String> = letters.iter().filter(|&&k| is_black(k)).map(|&k| note_name(k)).collect();
    if !black.is_empty() {
        facts.push(format!("Black keys: {}", black.join(", ")));
    }
    let mut lengths: Vec<f32> = music.events.iter().filter(|e| !e.pitches.is_empty()).map(|e| e.beats).collect();
    lengths.sort_by(f32::total_cmp);
    lengths.dedup();
    let name = |b: f32| match b {
        0.25 => "sixteenth (¼ beat)",
        0.5 => "eighth (½ beat)",
        0.75 => "dotted eighth (¾ beat)",
        1.0 => "quarter (1 beat)",
        1.5 => "dotted quarter (1½ beats)",
        2.0 => "half (2 beats)",
        3.0 => "dotted half (3 beats)",
        _ => "whole (4 beats)",
    };
    facts.push(format!("Note values: {}", lengths.iter().map(|&b| name(b)).collect::<Vec<_>>().join(", ")));
    if music.events.iter().any(|e| e.pitches.is_empty()) {
        facts.push("Rests: keep counting through the silence".into());
    }
    if music.events.iter().any(|e| e.pitches.len() > 1) {
        facts.push("Chords: press every key of a chord at the same moment".into());
    }

    let strings = |v: Vec<SharedString>| ModelRc::new(VecModel::from(v));
    app.set_side_keys(ModelRc::new(VecModel::from(keys)));
    app.set_side_whites(whites);
    app.set_side_hands(strings(lines));
    app.set_side_facts(strings(facts.into_iter().map(Into::into).collect()));
    app.set_side_tips(strings(tips.iter().map(|t| t.as_str().into()).collect()));
}

/// Pushes the illustration of a step, if it has one.
fn set_figure(app: &App, show: Option<&Show>) {
    let mut lit = vec![false; 5];
    let figure = match show {
        Some(Show::Keys(d)) => {
            let (keys, arcs, whites, rows) = diagram(d);
            app.set_diagram_keys(ModelRc::new(VecModel::from(keys)));
            app.set_diagram_arcs(ModelRc::new(VecModel::from(arcs)));
            app.set_diagram_whites(whites);
            app.set_diagram_rows(rows);
            "keys"
        }
        Some(Show::Hands(fingers)) => {
            for &f in fingers {
                lit[f as usize - 1] = true;
            }
            "hands"
        }
        Some(Show::Figure(name)) => name.as_str(),
        _ => "",
    };
    app.set_hands_lit(ModelRc::new(VecModel::from(lit)));
    app.set_figure(figure.into());
}

/// Lays out a key diagram: keys in white-key widths, and arcs stacked in rows so none overlap.
fn diagram(d: &KeyDiagram) -> (Vec<DiagramKey>, Vec<DiagramArc>, f32, i32) {
    let (from, to) = (d.from.midi(), d.to.midi());
    let mut whites = 0.0;
    let mut centre = HashMap::new();
    let keys: Vec<DiagramKey> = (from..=to)
        .map(|k| {
            let black = is_black(k);
            let x = whites; // a black key is centred on the boundary before the next white key
            centre.insert(k, if black { x } else { x + 0.5 });
            if !black {
                whites += 1.0;
            }
            let mark = d.marks.iter().find(|m| m.0.midi() == k);
            DiagramKey { x, black, label: mark.map_or("", |m| m.1.as_str()).into(), marked: mark.is_some(), note: k as i32, found: false, middle: k == 60, hand: 0 }
        })
        .collect();
    let mut arcs: Vec<DiagramArc> = d
        .arcs
        .iter()
        .map(|(a, b, label)| DiagramArc { x1: centre[&a.midi()], x2: centre[&b.midi()], label: label.as_str().into(), row: 0 })
        .collect();
    // Shortest arcs sit lowest; each goes in the first row where it overlaps nothing.
    arcs.sort_by(|a, b| (a.x1 - a.x2).abs().total_cmp(&(b.x1 - b.x2).abs()));
    let span = |a: &DiagramArc| (a.x1.min(a.x2), a.x1.max(a.x2));
    for i in 0..arcs.len() {
        let (lo, hi) = span(&arcs[i]);
        let mut row = 0;
        while arcs[..i].iter().any(|o| o.row == row && span(o).0 < hi && lo < span(o).1) {
            row += 1;
        }
        arcs[i].row = row;
    }
    let rows = arcs.iter().map(|a| a.row + 1).max().unwrap_or(0);
    (keys, arcs, whites, rows)
}

fn describe_latency(ms: f64) -> String {
    let side = if ms >= 0.0 { "after" } else { "before" };
    format!("about {:.0} ms {side} the click", ms.abs())
}

fn staff_of(show: &Show, names: bool) -> Option<Layout> {
    match show {
        Show::Staff(clef, pitches) => Some(notation::layout_pitches(*clef, pitches)),
        Show::Music(rh, lh) => Some(notation::layout(&Music::new(rh, lh), 0, names)),
        _ => None,
    }
}

fn set_staff(app: &App, layout: Option<Layout>) {
    let Some(Layout { items, width, height, .. }) = layout else {
        app.set_staff(ModelRc::new(VecModel::from(Vec::new())));
        return;
    };
    let items: Vec<StaffItem> = items
        .into_iter()
        .map(|item| {
            let mut s = StaffItem { x: item.x, y: item.y, note: item.note.map_or(-1, |n| n as i32), ..Default::default() };
            match item.shape {
                Shape::Rect { w, h } => (s.w, s.h) = (w, h),
                Shape::Glyph(c) => (s.kind, s.glyph) = (1, String::from(c).into()),
                Shape::Text { text, size } => (s.kind, s.glyph, s.size) = (2, text.into(), size),
                Shape::Brace { h } => (s.kind, s.h) = (3, h),
            }
            s
        })
        .collect();
    app.set_staff(ModelRc::new(VecModel::from(items)));
    app.set_staff_spaces(width);
    app.set_staff_rows(height);
}

/// Edits one key of the on-screen keyboard, touching the model only if something changed.
fn set_key(app: &App, note: u8, edit: impl Fn(&mut KeyState)) {
    let keys = app.get_keys();
    let row = note.wrapping_sub(LOW) as usize;
    if let Some(old) = keys.row_data(row) {
        let mut key = old.clone();
        edit(&mut key);
        if key != old {
            keys.set_row_data(row, key);
        }
    }
}
