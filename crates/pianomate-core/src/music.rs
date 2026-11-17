//! Music for both hands, written in a compact text form so lessons stay readable.
//!
//! A part is a list of tokens separated by spaces; `|` marks a barline for the reader and is ignored.
//!   C4        a note (letter, optional # or b, octave)
//!   C4/2      with its length in beats; a length carries over to the following notes until changed
//!   C4@1      with a finger number
//!   C4+E4+G4  a chord; fingers as @1+3+5
//!   r/2       a rest
//!   key=1     a key signature: sharps if positive, flats if negative (put it first)

use serde::Deserialize;

use crate::Pitch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Right,
    Left,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub hand: Hand,
    /// Start, in beats from the beginning.
    pub at: f32,
    pub beats: f32,
    /// Empty for a rest.
    pub pitches: Vec<Pitch>,
    /// One per pitch, or empty when no fingering is marked.
    pub fingers: Vec<u8>,
}

/// One hand's part, as written in a lesson file, and its key signature.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub struct Part(pub Vec<Event>, pub i8);

impl TryFrom<String> for Part {
    type Error = String;

    fn try_from(s: String) -> Result<Part, String> {
        // The hand is set when the part is placed in a `Music`.
        let key = s.split_whitespace().find_map(|t| t.strip_prefix("key=")).map(|k| k.parse::<i8>().ok().filter(|k| (-7..=7).contains(k)));
        let key = key.unwrap_or(Some(0)).ok_or_else(|| format!("{s:?}: key= takes -7 to 7 (flats negative, sharps positive)"))?;
        parse_part(&s, Hand::Right).map(|events| Part(events, key))
    }
}

pub const LENGTHS: [f32; 8] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0];

fn parse_part(src: &str, hand: Hand) -> Result<Vec<Event>, String> {
    let (mut events, mut at, mut beats) = (Vec::new(), 0.0, 1.0);
    for token in src.split_whitespace().filter(|t| *t != "|" && !t.starts_with("key=")) {
        let err = |why: &str| format!("{token:?}: {why}");
        let (body, fingers) = token.split_once('@').unwrap_or((token, ""));
        if token.ends_with('@') {
            return Err(err("give a finger number after @"));
        }
        let (notes, length) = body.split_once('/').unwrap_or((body, ""));
        if !length.is_empty() {
            beats = length.parse().map_err(|_| err("length must be a number of beats"))?;
            if !LENGTHS.contains(&beats) {
                return Err(err("length must be 0.25, 0.5, 0.75, 1, 1.5, 2, 3 or 4 beats"));
            }
        }
        let pitches: Vec<Pitch> = if notes == "r" {
            Vec::new()
        } else {
            notes.split('+').map(|p| p.parse()).collect::<Result<_, _>>().map_err(|e: String| err(&e))?
        };
        let fingers: Vec<u8> = if fingers.is_empty() {
            Vec::new()
        } else {
            fingers.split('+').map(|f| f.parse().ok().filter(|f| (1..=5).contains(f))).collect::<Option<_>>().ok_or_else(|| err("fingers are 1 to 5"))?
        };
        if !fingers.is_empty() && fingers.len() != pitches.len() {
            return Err(err("give one finger per note"));
        }
        events.push(Event { hand, at, beats, pitches, fingers });
        at += beats;
    }
    Ok(events)
}

/// Both hands together, ordered by time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Music {
    pub events: Vec<Event>,
    /// Key signature: sharps if positive, flats if negative.
    pub key: i8,
}

impl Music {
    pub fn new(rh: &Part, lh: &Part) -> Music {
        let mut events: Vec<Event> = rh.0.iter().cloned().chain(lh.0.iter().cloned().map(|e| Event { hand: Hand::Left, ..e })).collect();
        events.sort_by(|a, b| a.at.total_cmp(&b.at).then((a.hand as u8).cmp(&(b.hand as u8))));
        Music { events, key: if rh.1 != 0 { rh.1 } else { lh.1 } }
    }

    pub fn has(&self, hand: Hand) -> bool {
        self.events.iter().any(|e| e.hand == hand)
    }

    pub fn has_notes(&self) -> bool {
        self.events.iter().any(|e| !e.pitches.is_empty())
    }

    /// Events that must be played together, in order: indices into `events`, rests left out.
    pub fn moments(&self) -> Vec<Vec<usize>> {
        let mut moments: Vec<Vec<usize>> = Vec::new();
        let mut last_at = f32::NAN;
        for (i, e) in self.events.iter().enumerate().filter(|(_, e)| !e.pitches.is_empty()) {
            if e.at == last_at {
                moments.last_mut().unwrap().push(i);
            } else {
                moments.push(vec![i]);
                last_at = e.at;
            }
        }
        moments
    }

    /// The keys a hand plays, low to high, each with the first finger marked for it.
    pub fn fingering(&self, hand: Hand) -> Vec<(u8, Option<u8>)> {
        let mut keys: Vec<(u8, Option<u8>)> = Vec::new();
        for e in self.events.iter().filter(|e| e.hand == hand) {
            for (i, p) in e.pitches.iter().enumerate() {
                let finger = e.fingers.get(i).copied();
                match keys.iter_mut().find(|k| k.0 == p.midi()) {
                    Some(k) => k.1 = k.1.or(finger),
                    None => keys.push((p.midi(), finger)),
                }
            }
        }
        keys.sort_unstable();
        keys
    }

    /// Where a hand sits, in words: "thumb on C4", or the span it covers when no fingers are marked.
    pub fn position(&self, hand: Hand) -> Option<String> {
        let keys = self.fingering(hand);
        let (lo, hi) = (keys.first()?.0, keys.last()?.0);
        let name = crate::note_name;
        let at = |f: u8| keys.iter().find(|k| k.1 == Some(f)).map(|k| name(k.0));
        Some(match (at(1), at(5)) {
            (Some(thumb), Some(little)) => format!("thumb on {thumb}, little finger on {little}"),
            (Some(thumb), None) => format!("thumb on {thumb}"),
            (None, Some(little)) => format!("little finger on {little}"),
            _ if lo == hi => format!("plays {}", name(lo)),
            _ => format!("covers {} to {}", name(lo), name(hi)),
        })
    }

    /// The keys of a moment, without repeats.
    pub fn keys(&self, moment: &[usize]) -> Vec<u8> {
        let mut keys: Vec<u8> = moment.iter().flat_map(|&i| self.events[i].pitches.iter().map(|p| p.midi())).collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(s: &str) -> Part {
        Part::try_from(s.to_string()).unwrap()
    }

    #[test]
    fn parses_lengths_chords_rests_and_fingers() {
        let p = part("C4@1 D4/0.5 E4 | C4+E4+G4/2@1+3+5 r/1");
        let at: Vec<f32> = p.0.iter().map(|e| e.at).collect();
        let beats: Vec<f32> = p.0.iter().map(|e| e.beats).collect();
        assert_eq!(at, [0.0, 1.0, 1.5, 2.0, 4.0]);
        assert_eq!(beats, [1.0, 0.5, 0.5, 2.0, 1.0]);
        assert_eq!(p.0[0].fingers, [1]);
        assert_eq!(p.0[3].pitches.len(), 3);
        assert_eq!(p.0[3].fingers, [1, 3, 5]);
        assert!(p.0[4].pitches.is_empty());
    }

    #[test]
    fn rejects_bad_tokens() {
        for bad in ["H4", "C4/5", "C4/x", "C4@6", "C4+E4@1", "C4@"] {
            assert!(Part::try_from(bad.to_string()).is_err(), "{bad:?} should be rejected");
        }
        assert_eq!(part("").0, []);
        assert_eq!(part("key=1 G4 F#4").1, 1);
        assert_eq!(part("key=1 G4 F#4").0.len(), 2);
        assert!(Part::try_from("key=9 G4".to_string()).is_err());
    }

    #[test]
    fn hand_positions() {
        let m = Music::new(&part("C4/1@1 D4 E4@3 G4@5 C4"), &part("C3+E3+G3/4@5+3+1"));
        assert_eq!(m.fingering(Hand::Right), [(60, Some(1)), (62, None), (64, Some(3)), (67, Some(5))]);
        assert_eq!(m.position(Hand::Right).unwrap(), "thumb on C4, little finger on G4");
        assert_eq!(m.position(Hand::Left).unwrap(), "thumb on G3, little finger on C3");
        let plain = Music::new(&part("E4 D4 C4"), &Part::default());
        assert_eq!(plain.position(Hand::Right).unwrap(), "covers C4 to E4");
        assert_eq!(plain.position(Hand::Left), None);
    }

    #[test]
    fn hands_merge_into_moments() {
        let m = Music::new(&part("E4 D4 C4/2"), &part("C3/2 C3/2"));
        assert!(m.has(Hand::Left) && m.has(Hand::Right));
        let moments = m.moments();
        assert_eq!(moments.len(), 3, "C3 with E4, D4 alone, C4 with C3");
        assert_eq!(m.keys(&moments[0]), [48, 64]);
        assert_eq!(m.keys(&moments[1]), [62]);
        assert_eq!(m.keys(&moments[2]), [48, 60]);
        assert_eq!(m.events[0].hand, Hand::Right, "at the same moment the right hand sorts first");
    }
}
