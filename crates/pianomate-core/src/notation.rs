//! Lays music out as a display list the UI just paints.
//!
//! Units are staff spaces; y grows downward from the top of the drawing. Glyphs are SMuFL code
//! points (Bravura font) positioned by their baseline origin.

use crate::{
    Pitch,
    lesson::Clef,
    music::{Event, Hand, Music, Part},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub x: f32,
    pub y: f32,
    pub shape: Shape,
    /// Index of the event this belongs to (for colouring); `None` for the staff itself.
    pub note: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// x is the left edge, y the baseline.
    Glyph(char),
    /// x, y is the top-left corner.
    Rect { w: f32, h: f32 },
    /// Plain text centred on x, y: finger numbers and letter names.
    Text { text: String, size: f32 },
    /// The curly brace joining a grand staff, from y down `h` spaces.
    Brace { h: f32 },
}

pub struct Layout {
    pub items: Vec<Item>,
    pub width: f32,
    pub height: f32,
    /// (beat, x) pairs, ascending: where each moment in time sits, for a playhead.
    pub time: Vec<(f32, f32)>,
}

impl Layout {
    /// Horizontal position of a moment `beat` beats in, between the known points.
    pub fn x_at(&self, beat: f32) -> f32 {
        let after = self.time.iter().position(|&(b, _)| b > beat).unwrap_or(self.time.len());
        match (after.checked_sub(1).map(|i| self.time[i]), self.time.get(after)) {
            (Some((b0, x0)), Some(&(b1, x1))) => x0 + (x1 - x0) * (beat - b0) / (b1 - b0),
            (Some((_, x)), None) | (None, Some(&(_, x))) => x,
            (None, None) => 0.0,
        }
    }
}

const G_CLEF: char = '\u{E050}';
const F_CLEF: char = '\u{E062}';
const PERCUSSION_CLEF: char = '\u{E069}';
const TIME_DIGIT_0: u32 = 0xE080;
const HEAD_WHOLE: char = '\u{E0A2}';
const HEAD_HALF: char = '\u{E0A3}';
const HEAD_BLACK: char = '\u{E0A4}';
const DOT: char = '\u{E1E7}';
const FLAG_UP: char = '\u{E240}';
const FLAG_DOWN: char = '\u{E241}';
const FLAG16_UP: char = '\u{E242}';
const FLAG16_DOWN: char = '\u{E243}';
const FLAT: char = '\u{E260}';
const NATURAL: char = '\u{E261}';
const SHARP: char = '\u{E262}';
const REST_WHOLE: char = '\u{E4E3}';
const REST_HALF: char = '\u{E4E4}';
const REST_QUARTER: char = '\u{E4E5}';
const REST_8TH: char = '\u{E4E6}';
const REST_16TH: char = '\u{E4E7}';

const LINE: f32 = 0.1;
const STEM: f32 = 0.12;
const STEM_LEN: f32 = 3.5;
/// Room above the top staff and below the bottom one, for ledger lines and finger numbers.
const MARGIN: f32 = 5.0;
/// Space between the treble and bass staves of a grand staff.
const STAFF_GAP: f32 = 6.0;
const HEAD_W: f32 = 1.18;
/// Rhythm staves are spaced in proportion to time, so a playhead can sweep across them.
const RHYTHM_FIRST_NOTE: f32 = 6.5;
const RHYTHM_BEAT: f32 = 3.5;

fn rect(x: f32, y: f32, w: f32, h: f32, note: Option<usize>) -> Item {
    Item { x, y, shape: Shape::Rect { w, h }, note }
}

fn glyph(x: f32, y: f32, c: char, note: Option<usize>) -> Item {
    Item { x, y, shape: Shape::Glyph(c), note }
}

fn text(x: f32, y: f32, s: String, size: f32, note: Option<usize>) -> Item {
    Item { x, y, shape: Shape::Text { text: s, size }, note }
}

/// Vertical position of a pitch on its staff: 0 = top line, 0.5 = the space below it, and so on.
pub fn staff_y(clef: Clef, pitch: Pitch) -> f32 {
    // Diatonic step of the bottom line: E4 in treble, G2 in bass.
    let bottom = match clef {
        Clef::Treble => 4 * 7 + 2,
        Clef::Bass => 2 * 7 + 4,
    };
    4.0 - (pitch.step - bottom) as f32 / 2.0
}

fn head_for(beats: f32) -> (char, f32) {
    match beats {
        b if b >= 4.0 => (HEAD_WHOLE, 1.69),
        b if b >= 2.0 => (HEAD_HALF, HEAD_W),
        _ => (HEAD_BLACK, HEAD_W),
    }
}

/// Letters (C = 0 … B = 6) that a key signature alters, in the order they are written.
const SHARP_ORDER: [i8; 7] = [3, 0, 4, 1, 5, 2, 6]; // F C G D A E B
const FLAT_ORDER: [i8; 7] = [6, 2, 5, 1, 4, 0, 3]; // B E A D G C F

/// How the key signature alters a letter: +1, -1 or 0.
fn key_alter(key: i8, letter: i8) -> i8 {
    let order = if key > 0 { &SHARP_ORDER } else { &FLAT_ORDER };
    if order[..key.unsigned_abs() as usize].contains(&letter) { key.signum() } else { 0 }
}

/// The accidental to write before a pitch in this key, if any.
// shortcut: every note gets its own accidental (none carry through the bar); fine for short exercises.
fn accidental(key: i8, p: Pitch) -> Option<char> {
    (p.alter != key_alter(key, p.step % 7)).then_some(match p.alter {
        1 => SHARP,
        -1 => FLAT,
        _ => NATURAL,
    })
}

/// Treble-clef positions of a key signature's sharps and flats, as diatonic steps.
const SHARP_STEPS: [i8; 7] = [38, 35, 39, 36, 33, 37, 34]; // F5 C5 G5 D5 A4 E5 B4
const FLAT_STEPS: [i8; 7] = [34, 37, 33, 36, 32, 35, 31]; // B4 E5 A4 D5 G4 C5 F4

fn dotted(beats: f32) -> bool {
    [0.75, 1.5, 3.0].contains(&beats)
}

/// Draws the notes of one event (a note or chord) at x on a staff whose top line is at `top`.
// shortcut: flags, never beams; seconds in a chord always push the upper note right; add proper
// engraving when pieces get denser.
fn chord(items: &mut Vec<Item>, x: f32, top: f32, clef: Clef, key: i8, e: &Event, names: bool, idx: Option<usize>) {
    let (head, head_w) = head_for(e.beats);
    let mut ys: Vec<(f32, Pitch)> = e.pitches.iter().map(|&p| (top + staff_y(clef, p), p)).collect();
    ys.sort_by(|a, b| b.0.total_cmp(&a.0)); // lowest note first
    let mid = top + 2.0;
    let stem_up = ys.iter().map(|y| y.0).sum::<f32>() / ys.len() as f32 > mid;

    let mut last_shifted = false;
    for (k, &(y, pitch)) in ys.iter().enumerate() {
        let second = k > 0 && (ys[k - 1].0 - y - 0.5).abs() < 0.01 && !last_shifted;
        last_shifted = second;
        let hx = if second { x + head_w } else { x };
        if let Some(sign) = accidental(key, pitch) {
            items.push(glyph(x - 1.25, y, sign, idx));
        }
        items.push(glyph(hx, y, head, idx));
        if dotted(e.beats) {
            // A dot never sits on a line: nudge it into the space above.
            let on_line = (y - top).fract() == 0.0;
            items.push(glyph(x + head_w + 0.35, if on_line { y - 0.5 } else { y }, DOT, idx));
        }
        // Ledger lines: every whole line between the staff and the note.
        let (from, step) = if y > top + 4.0 { (top + 5.0, 1.0) } else { (top - 1.0, -1.0) };
        let mut ledger = from;
        while (y > top + 4.0 && ledger <= y) || (y < top && ledger >= y) {
            items.push(rect(hx - 0.4, ledger - 0.08, head_w + 0.8, 0.16, idx));
            ledger += step;
        }
    }

    let (low, high) = (ys[0].0, ys[ys.len() - 1].0);
    let (stem_top, stem_bottom) = if head == HEAD_WHOLE {
        (high, low)
    } else if stem_up {
        let sx = x + head_w - STEM;
        items.push(rect(sx, high - STEM_LEN, STEM, low - high + STEM_LEN - 0.17, idx));
        if e.beats < 1.0 {
            items.push(glyph(sx, high - STEM_LEN, if e.beats < 0.5 { FLAG16_UP } else { FLAG_UP }, idx));
        }
        (high - STEM_LEN, low)
    } else {
        items.push(rect(x, high + 0.17, STEM, low - high + STEM_LEN - 0.17, idx));
        if e.beats < 1.0 {
            items.push(glyph(x, low + STEM_LEN, if e.beats < 0.5 { FLAG16_DOWN } else { FLAG_DOWN }, idx));
        }
        (high, low + STEM_LEN)
    };

    // Letter names in a row of their own, clear of the noteheads: below the right hand's staff and
    // above the left hand's, the side away from the finger numbers.
    if names {
        let mut letters: Vec<(f32, Pitch)> = ys.clone();
        let cx = x + head_w / 2.0;
        let name = |p: Pitch| format!("{}{}", "CDEFGAB".chars().nth((p.step % 7) as usize).unwrap(), ["♭", "", "♯"][(p.alter + 1) as usize]);
        match e.hand {
            Hand::Right => {
                letters.sort_by(|a, b| a.0.total_cmp(&b.0)); // top note first, reading down
                let row = (top + 6.4).max(low + 1.7);
                for (k, (_, p)) in letters.iter().enumerate() {
                    items.push(text(cx, row + k as f32 * 1.25, name(*p), 0.95, idx));
                }
            }
            Hand::Left => {
                let row = (top - 2.4).min(high - 1.7);
                for (k, (_, p)) in letters.iter().enumerate() {
                    items.push(text(cx, row - k as f32 * 1.25, name(*p), 0.95, idx));
                }
            }
        }
    }

    // Finger numbers: above the right hand, below the left, in the order the notes are stacked.
    if !e.fingers.is_empty() {
        let mut fingers: Vec<(f32, u8)> = e.pitches.iter().map(|&p| top + staff_y(clef, p)).zip(e.fingers.iter().copied()).collect();
        fingers.sort_by(|a, b| a.0.total_cmp(&b.0)); // highest note first
        let cx = x + head_w / 2.0;
        match e.hand {
            Hand::Right => {
                let above = stem_top.min(top) - 1.0;
                for (k, (_, f)) in fingers.iter().rev().enumerate() {
                    items.push(text(cx, above - k as f32 * 1.3, f.to_string(), 1.05, idx));
                }
            }
            Hand::Left => {
                let below = stem_bottom.max(top + 4.0) + 1.1;
                for (k, (_, f)) in fingers.iter().enumerate() {
                    items.push(text(cx, below + k as f32 * 1.3, f.to_string(), 1.05, idx));
                }
            }
        }
    }
}

fn rest(items: &mut Vec<Item>, x: f32, top: f32, beats: f32) {
    let (c, y) = match beats {
        b if b >= 4.0 => (REST_WHOLE, top + 1.0),
        b if b >= 2.0 => (REST_HALF, top + 2.0),
        b if b >= 1.0 => (REST_QUARTER, top + 2.0),
        b if b >= 0.5 => (REST_8TH, top + 2.0),
        _ => (REST_16TH, top + 2.0),
    };
    items.push(glyph(x, y, c, None));
    if dotted(beats) {
        items.push(glyph(x + 1.6, top + 1.5, DOT, None));
    }
}

/// Room a moment takes, from the time until the next one: longer notes get more space.
fn advance(beats: f32) -> f32 {
    2.4 + 1.9 * beats.sqrt()
}

/// Music on one staff, or a grand staff when both hands play.
// shortcut: everything on one line, no system breaks; keep exercises to about four bars until
// pieces need wrapping.
pub fn layout(music: &Music, beats_per_bar: u8, names: bool) -> Layout {
    let grand = music.has(Hand::Left) && music.has(Hand::Right);
    // Each hand's staff: its top line and clef.
    let staff = |hand: Hand| match (grand, hand) {
        (_, Hand::Right) => (MARGIN, Clef::Treble),
        (true, Hand::Left) => (MARGIN + 4.0 + STAFF_GAP, Clef::Bass),
        (false, Hand::Left) => (MARGIN, Clef::Bass),
    };
    let staves: Vec<(f32, Clef)> = if grand {
        vec![staff(Hand::Right), staff(Hand::Left)]
    } else {
        vec![staff(if music.has(Hand::Left) { Hand::Left } else { Hand::Right })]
    };
    let (system_top, system_bottom) = (MARGIN, staves.last().unwrap().0 + 4.0);

    let mut items = Vec::new();
    // A grand staff's lines start just right of its brace.
    let x0 = if grand { 1.7 } else { 0.0 };
    let mut x = x0 + 0.4;
    if grand {
        items.push(Item { x: 0.0, y: system_top, shape: Shape::Brace { h: system_bottom - system_top }, note: None });
    }
    for &(top, clef) in &staves {
        items.push(match clef {
            Clef::Treble => glyph(x + 0.4, top + 3.0, G_CLEF, None),
            Clef::Bass => glyph(x + 0.4, top + 1.0, F_CLEF, None),
        });
    }
    x += 3.6;
    if music.key != 0 {
        let (steps, sign) = if music.key > 0 { (&SHARP_STEPS, SHARP) } else { (&FLAT_STEPS, FLAT) };
        for &(top, clef) in &staves {
            let shift = if clef == Clef::Bass { 14 } else { 0 };
            for (i, &step) in steps[..music.key.unsigned_abs() as usize].iter().enumerate() {
                items.push(glyph(x + i as f32, top + staff_y(clef, Pitch { step: step - shift, alter: 0 }), sign, None));
            }
        }
        x += music.key.unsigned_abs() as f32 + 0.8;
    }
    if beats_per_bar > 0 {
        for &(top, _) in &staves {
            items.push(glyph(x, top + 1.0, char::from_u32(TIME_DIGIT_0 + beats_per_bar as u32).unwrap(), None));
            items.push(glyph(x, top + 3.0, char::from_u32(TIME_DIGIT_0 + 4).unwrap(), None));
        }
        x += 2.6;
    }
    x += 1.2;

    // Moments in time, rests included, so both hands line up.
    let mut onsets: Vec<f32> = music.events.iter().map(|e| e.at).collect();
    onsets.dedup();
    let end = music.events.iter().map(|e| e.at + e.beats).fold(0.0, f32::max);
    let mut time = Vec::new();
    for (k, &at) in onsets.iter().enumerate() {
        if beats_per_bar > 0 && at > 0.0 && at % beats_per_bar as f32 == 0.0 {
            items.push(rect(x - 1.3, system_top, LINE * 1.2, system_bottom - system_top, None));
        }
        let here: Vec<(usize, &Event)> = music.events.iter().enumerate().filter(|(_, e)| e.at == at).collect();
        if here.iter().any(|(_, e)| e.pitches.iter().any(|p| accidental(music.key, *p).is_some())) {
            x += 1.0;
        }
        time.push((at, x + HEAD_W / 2.0));
        for (i, e) in here {
            let (top, clef) = staff(e.hand);
            if e.pitches.is_empty() {
                rest(&mut items, x, top, e.beats);
            } else {
                chord(&mut items, x, top, clef, music.key, e, names, Some(i));
            }
        }
        x += advance(onsets.get(k + 1).unwrap_or(&end) - at);
    }
    let width = x - 0.6;
    time.push((end, width - 0.4));

    for &(top, _) in &staves {
        items.extend((0..5).map(|i| rect(x0, top + i as f32 - LINE / 2.0, width - x0, LINE, None)));
    }
    if grand {
        items.push(rect(x0, system_top, LINE * 1.2, system_bottom - system_top, None));
    }
    items.push(rect(width - 0.16, system_top - LINE / 2.0, 0.16, system_bottom - system_top + LINE, None));
    Layout { items, width, height: system_bottom + MARGIN, time }
}

/// Pitches shown as whole notes on one staff, with no rhythm implied.
pub fn layout_pitches(clef: Clef, pitches: &[Pitch]) -> Layout {
    let token = |p: &Pitch| format!("{}{}{}/4", "CDEFGAB".chars().nth((p.step % 7) as usize).unwrap(), ["b", "", "#"][(p.alter + 1) as usize], p.step / 7);
    let part = Part::try_from(pitches.iter().map(token).collect::<Vec<_>>().join(" ")).unwrap();
    let empty = Part::default();
    let music = match clef {
        Clef::Treble => Music::new(&part, &empty),
        Clef::Bass => Music::new(&empty, &part),
    };
    layout(&music, 0, false)
}

/// Horizontal position on a rhythm staff of a moment `beat` beats into the pattern.
pub fn rhythm_x(beat: f32) -> f32 {
    RHYTHM_FIRST_NOTE + beat * RHYTHM_BEAT + 0.59 // centre of a notehead
}

/// A rhythm on a one-line staff with `beats_per_bar` quarter-note beats to the bar.
pub fn layout_rhythm(beats_per_bar: u8, beats: &[f32]) -> Layout {
    let total: f32 = beats.iter().sum();
    let width = RHYTHM_FIRST_NOTE + total * RHYTHM_BEAT + 0.5;
    let line = MARGIN + 2.0;
    let mut items = vec![
        rect(0.0, line - LINE / 2.0, width, LINE, None),
        rect(width - 0.16, line - 1.0, 0.16, 2.0, None),
        glyph(0.6, line, PERCUSSION_CLEF, None),
        glyph(3.2, line - 1.1, char::from_u32(TIME_DIGIT_0 + beats_per_bar as u32 % 10).unwrap(), None),
        glyph(3.2, line + 1.1, char::from_u32(TIME_DIGIT_0 + 4).unwrap(), None),
    ];
    let mut at = 0.0;
    for (i, &b) in beats.iter().enumerate() {
        let x = RHYTHM_FIRST_NOTE + at * RHYTHM_BEAT;
        if at > 0.0 && at % beats_per_bar as f32 == 0.0 {
            items.push(rect(x - 1.0, line - 1.0, LINE, 2.0, None));
        }
        let (head, _) = head_for(b);
        items.push(glyph(x, line, head, Some(i)));
        if dotted(b) {
            items.push(glyph(x + HEAD_W + 0.35, line - 0.5, DOT, Some(i)));
        }
        if head != HEAD_WHOLE {
            items.push(rect(x + HEAD_W - STEM, line - STEM_LEN, STEM, STEM_LEN - 0.17, Some(i)));
            if b < 1.0 {
                items.push(glyph(x + HEAD_W - STEM, line - STEM_LEN, FLAG_UP, Some(i)));
            }
        }
        at += b;
    }
    Layout { items, width, height: line + 2.0 + MARGIN, time: vec![(0.0, rhythm_x(0.0)), (total, rhythm_x(total))] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Pitch {
        s.parse().unwrap()
    }

    fn part(s: &str) -> Part {
        Part::try_from(s.to_string()).unwrap()
    }

    fn count(items: &[Item], shape: impl Fn(&Shape) -> bool) -> usize {
        items.iter().filter(|i| shape(&i.shape)).count()
    }

    #[test]
    fn staff_positions() {
        assert_eq!(staff_y(Clef::Treble, p("E4")), 4.0);
        assert_eq!(staff_y(Clef::Treble, p("F5")), 0.0);
        assert_eq!(staff_y(Clef::Treble, p("C4")), 5.0);
        assert_eq!(staff_y(Clef::Bass, p("G2")), 4.0);
        assert_eq!(staff_y(Clef::Bass, p("C3")), 2.5);
        assert_eq!(staff_y(Clef::Bass, p("C4")), -1.0, "middle C is one ledger line above the bass staff");
    }

    fn ledgers(clef: Clef, pitch: &str) -> Vec<f32> {
        // Ledger lines are the only rects 0.16 tall.
        layout_pitches(clef, &[p(pitch)])
            .items
            .iter()
            .filter(|i| matches!(i.shape, Shape::Rect { h, .. } if h == 0.16) && i.note.is_some())
            .map(|i| i.y + 0.08 - MARGIN)
            .collect()
    }

    #[test]
    fn ledger_lines() {
        assert_eq!(ledgers(Clef::Treble, "C4"), [5.0]);
        assert_eq!(ledgers(Clef::Treble, "D4"), [] as [f32; 0]);
        assert_eq!(ledgers(Clef::Treble, "A3"), [5.0, 6.0]);
        assert_eq!(ledgers(Clef::Treble, "A5"), [-1.0]);
        assert_eq!(ledgers(Clef::Treble, "G5"), [] as [f32; 0]);
        assert_eq!(ledgers(Clef::Bass, "C4"), [-1.0]);
    }

    #[test]
    fn note_lengths() {
        let parts = |len: &str| {
            let l = layout(&Music::new(&part(&format!("G4/{len}")), &Part::default()), 0, false);
            (
                count(&l.items, |s| matches!(s, Shape::Rect { w, .. } if *w == STEM)),
                count(&l.items, |s| matches!(s, Shape::Glyph(FLAG_UP | FLAG16_UP))),
                count(&l.items, |s| *s == Shape::Glyph(DOT)),
            )
        };
        // (stems, flags, dots)
        assert_eq!(parts("4"), (0, 0, 0));
        assert_eq!(parts("3"), (1, 0, 1));
        assert_eq!(parts("2"), (1, 0, 0));
        assert_eq!(parts("1.5"), (1, 0, 1));
        assert_eq!(parts("1"), (1, 0, 0));
        assert_eq!(parts("0.5"), (1, 1, 0));
        assert_eq!(parts("0.25"), (1, 1, 0));
    }

    #[test]
    fn grand_staff_aligns_hands_and_marks_bars() {
        let music = Music::new(&part("E4@3 D4 C4/2 | E4/4"), &part("C3+G3/2@5+1 r/2 | C3/4"));
        let l = layout(&music, 4, true);
        let x_of = |event: usize| l.items.iter().find(|i| i.note == Some(event) && matches!(i.shape, Shape::Glyph(_))).unwrap().x;
        // Events sort by time, right hand first: E4, C3+G3, D4, C4, rest, E4, C3.
        assert_eq!(music.events[1].hand, Hand::Left);
        assert_eq!(x_of(0), x_of(1), "notes starting together line up across the staves");
        assert_eq!(count(&l.items, |s| matches!(s, Shape::Brace { .. })), 1);
        assert_eq!(count(&l.items, |s| *s == Shape::Glyph(REST_HALF)), 1);
        assert_eq!(count(&l.items, |s| matches!(s, Shape::Text { text, size, .. } if *size > 1.0 && text == "5")), 1, "finger 5 printed once");
        let tall = l.height - 2.0 * MARGIN;
        assert_eq!(count(&l.items, |s| matches!(s, Shape::Rect { h, w } if *h == tall && *w == LINE * 1.2)), 2, "start line and one barline");
        assert_eq!(count(&l.items, |s| matches!(s, Shape::Text { size, .. } if *size < 1.0)), 7, "a letter name for each of the seven notes");
        let below_treble = l.items.iter().filter(|i| i.note == Some(0) && matches!(i.shape, Shape::Text { size, .. } if size < 1.0)).all(|i| i.y > MARGIN + 4.0);
        assert!(below_treble, "right-hand letters sit below the treble staff, clear of the notes");
    }

    #[test]
    fn playhead_follows_the_notes() {
        let l = layout(&Music::new(&part("C4/1 D4 E4/2"), &Part::default()), 4, false);
        assert_eq!(l.time.len(), 4, "three onsets and the end");
        assert_eq!(l.x_at(0.0), l.time[0].1);
        let half = l.x_at(0.5);
        assert!(l.time[0].1 < half && half < l.time[1].1);
        assert_eq!(l.x_at(99.0), l.time[3].1);
        assert_eq!(layout_rhythm(4, &[1.0, 1.0]).x_at(1.0), rhythm_x(1.0));
    }

    #[test]
    fn key_signatures() {
        let sharps = |src: &str| {
            let l = layout(&Music::new(&part(src), &Part::default()), 0, false);
            (count(&l.items, |s| *s == Shape::Glyph(SHARP)), count(&l.items, |s| *s == Shape::Glyph(NATURAL)))
        };
        assert_eq!(sharps("F#4 G4"), (1, 0), "no key: the sharp is written");
        assert_eq!(sharps("key=1 F#4 G4"), (1, 0), "in G the signature's sharp replaces the note's");
        assert_eq!(sharps("key=1 F4"), (1, 1), "an F in G major needs a natural");
        assert_eq!(sharps("key=2 C#5 F#4"), (2, 0));
        assert_eq!(key_alter(-1, 6), -1, "F major flattens B");
        assert_eq!(key_alter(-1, 2), 0);
    }

    #[test]
    fn rhythm_is_spaced_by_time_with_barlines() {
        let l = layout_rhythm(4, &[1.0, 1.0, 2.0, 0.5, 0.5, 1.0, 2.0]);
        let heads: Vec<f32> = l.items.iter().filter(|i| matches!(i.shape, Shape::Glyph(HEAD_BLACK | HEAD_HALF))).map(|i| i.x).collect();
        assert_eq!(heads.len(), 7);
        assert_eq!(heads[2] - heads[1], RHYTHM_BEAT);
        assert_eq!(heads[4] - heads[3], RHYTHM_BEAT / 2.0);
        assert_eq!(rhythm_x(0.0), heads[0] + 0.59);
        assert_eq!(count(&l.items, |s| matches!(s, Shape::Rect { w, h } if *w == LINE && *h == 2.0)), 1);
    }
}
