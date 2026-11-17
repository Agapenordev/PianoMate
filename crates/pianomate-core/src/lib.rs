//! Pure music logic: no I/O, no UI. Keys are MIDI numbers (60 = middle C).

pub mod curriculum;
pub mod drill;
pub mod lesson;
pub mod music;
pub mod notation;
pub mod rhythm;

use serde::Deserialize;

pub fn is_black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// Computer-keyboard fallback: home row = white keys from middle C, row above = black keys.
// shortcut: matches letters, so the piano shape only lines up on QWERTY; upgrade to scancodes if Slint exposes them.
pub const QWERTY: &str = "awsedftgyhujkolp;";

pub fn qwerty_note(c: char) -> Option<u8> {
    QWERTY.find(c.to_ascii_lowercase()).map(|i| 60 + i as u8)
}

pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
    format!("{}{}", NAMES[(note % 12) as usize], note as i32 / 12 - 1)
}

/// A written pitch: keeps its spelling (F♯ vs G♭) because the staff needs it.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub struct Pitch {
    /// Diatonic steps above C0: octave * 7 + letter (C = 0 … B = 6).
    pub step: i8,
    pub alter: i8,
}

impl Pitch {
    pub fn midi(self) -> u8 {
        (12 * (self.step / 7 + 1) + [0, 2, 4, 5, 7, 9, 11][(self.step % 7) as usize] + self.alter) as u8
    }
}

impl std::str::FromStr for Pitch {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let err = || format!("bad pitch {s:?}: expected e.g. C4, F#3 or Bb5");
        let mut chars = s.chars();
        let letter = chars.next().and_then(|c| "CDEFGAB".find(c)).ok_or_else(err)? as i8;
        let rest = chars.as_str();
        let (alter, octave) = if let Some(r) = rest.strip_prefix('#') {
            (1, r)
        } else if let Some(r) = rest.strip_prefix('b') {
            (-1, r)
        } else {
            (0, rest)
        };
        match octave.parse::<i8>() {
            Ok(octave @ 0..=8) => Ok(Pitch { step: octave * 7 + letter, alter }),
            _ => Err(err()),
        }
    }
}

impl TryFrom<String> for Pitch {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        s.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piano_has_52_white_keys() {
        assert_eq!((21..=108).filter(|&n| !is_black(n)).count(), 52);
    }

    #[test]
    fn qwerty_mapping() {
        assert_eq!(qwerty_note('a'), Some(60));
        assert_eq!(qwerty_note('K'), Some(72));
        assert_eq!(qwerty_note('z'), None);
        for c in "wetyuop".chars() {
            assert!(is_black(qwerty_note(c).unwrap()), "{c} should be a black key");
        }
        for c in "asdfghjkl;".chars() {
            assert!(!is_black(qwerty_note(c).unwrap()), "{c} should be a white key");
        }
    }

    #[test]
    fn pitches() {
        let midi = |s: &str| s.parse::<Pitch>().map(Pitch::midi);
        assert_eq!(midi("C4"), Ok(60));
        assert_eq!(midi("F#3"), Ok(54));
        assert_eq!(midi("Bb5"), Ok(82));
        assert_eq!(midi("A0"), Ok(21));
        assert_eq!(midi("C8"), Ok(108));
        for bad in ["", "H4", "c4", "C", "C9", "C#", "C4x"] {
            assert!(midi(bad).is_err(), "{bad:?} should be rejected");
        }
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(21), "A0");
    }
}
