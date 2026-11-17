//! Grades taps against a rhythm. Times are milliseconds from the first note of the pattern,
//! measured by the caller on one clock.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    Pending,
    Good,
    Early,
    Late,
    Missed,
}

pub struct RhythmRun {
    onsets: Vec<f64>,
    /// One mark per note of the pattern.
    pub marks: Vec<Mark>,
    /// Taps that matched no note.
    pub extra: usize,
    tolerance: f64,
    /// A tap further than this from every open note belongs to none of them.
    window: f64,
    end: f64,
}

impl RhythmRun {
    /// `beats` are note lengths in beats; a tap within `tolerance_ms` of its note is in time.
    pub fn new(beats: &[f32], bpm: f32, tolerance_ms: f32) -> Self {
        let beat_ms = 60_000.0 / bpm as f64;
        let mut end = 0.0;
        let onsets: Vec<f64> = beats
            .iter()
            .map(|&b| {
                let at = end;
                end += b as f64 * beat_ms;
                at
            })
            .collect();
        Self::with_onsets(onsets, end, tolerance_ms)
    }

    /// Notes starting at the given times (ms, ascending), the pattern lasting until `end`.
    pub fn with_onsets(onsets: Vec<f64>, end: f64, tolerance_ms: f32) -> Self {
        // Half the shortest gap: beyond that a tap is nearer to a neighbouring note.
        let gaps = onsets.windows(2).map(|w| w[1] - w[0]).chain([end - onsets.last().copied().unwrap_or(0.0)]);
        let window = gaps.fold(f64::MAX, f64::min).min(600.0) / 2.0;
        RhythmRun { marks: vec![Mark::Pending; onsets.len()], onsets, extra: 0, tolerance: (tolerance_ms as f64).min(window), window, end }
    }

    /// Length of the whole pattern.
    pub fn end_ms(&self) -> f64 {
        self.end
    }

    pub fn window_ms(&self) -> f64 {
        self.window
    }

    /// A tap at time `t`. Returns the mark it earned, or `None` if it matched no note.
    pub fn tap(&mut self, t: f64) -> Option<Mark> {
        if t < -self.window {
            return None; // tapping along with the count-in is fine
        }
        let Some(i) = self.nearest(t, |_| true) else {
            self.extra += 1;
            return None;
        };
        Some(self.mark(i, t))
    }

    /// The open note nearest to `t` within reach, among those `allowed`.
    pub fn nearest(&self, t: f64, allowed: impl Fn(usize) -> bool) -> Option<usize> {
        (0..self.onsets.len())
            .filter(|&i| self.marks[i] == Mark::Pending && (t - self.onsets[i]).abs() <= self.window && allowed(i))
            .min_by(|&a, &b| (t - self.onsets[a]).abs().total_cmp(&(t - self.onsets[b]).abs()))
    }

    /// Whether `t` is within reach of note `i`, open or not.
    pub fn near(&self, i: usize, t: f64) -> bool {
        (t - self.onsets[i]).abs() <= self.window
    }

    /// Grades note `i` as played at `t`.
    pub fn mark(&mut self, i: usize, t: f64) -> Mark {
        let offset = t - self.onsets[i];
        self.marks[i] = if offset.abs() <= self.tolerance {
            Mark::Good
        } else if offset < 0.0 {
            Mark::Early
        } else {
            Mark::Late
        };
        self.marks[i]
    }

    /// Time has reached `t`: notes whose window has passed untapped are missed.
    pub fn expire(&mut self, t: f64) {
        for (mark, onset) in self.marks.iter_mut().zip(&self.onsets) {
            if *mark == Mark::Pending && onset + self.window < t {
                *mark = Mark::Missed;
            }
        }
    }

    pub fn passed(&self) -> bool {
        self.extra == 0 && self.marks.iter().all(|&m| m == Mark::Good)
    }
}

/// The steady offset of taps from a click with period `beat_ms` that started at time 0:
/// positive when taps land after the click. Used to calibrate out device latency.
pub fn median_offset(taps: &[f64], beat_ms: f64) -> f64 {
    let mut offsets: Vec<f64> = taps.iter().map(|t| (t + beat_ms / 2.0).rem_euclid(beat_ms) - beat_ms / 2.0).collect();
    offsets.sort_by(f64::total_cmp);
    offsets.get(offsets.len() / 2).copied().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 120 bpm: a beat is 500 ms. Pattern: quarter, two eighths, quarter → onsets 0, 500, 750, 1000.
    fn run() -> RhythmRun {
        RhythmRun::new(&[1.0, 0.5, 0.5, 1.0], 120.0, 80.0)
    }

    #[test]
    fn in_time_passes() {
        let mut r = run();
        assert_eq!(r.end_ms(), 1500.0);
        assert_eq!(r.tap(-900.0), None, "count-in taps are ignored");
        for t in [20.0, 470.0, 760.0, 1079.0] {
            assert_eq!(r.tap(t), Some(Mark::Good));
        }
        r.expire(f64::INFINITY);
        assert!(r.passed());
    }

    #[test]
    fn early_late_missed_and_extra() {
        let mut r = run();
        assert_eq!(r.tap(-100.0), Some(Mark::Early));
        assert_eq!(r.tap(600.0), Some(Mark::Late), "100 ms after the note at 500");
        assert_eq!(r.tap(640.0), Some(Mark::Early), "the note at 500 is taken, so this is 110 ms early for 750");
        r.expire(900.0);
        assert_eq!(r.marks, [Mark::Early, Mark::Late, Mark::Early, Mark::Pending]);
        assert_eq!(r.tap(1300.0), None, "too far from any open note");
        assert_eq!(r.extra, 1);
        r.expire(f64::INFINITY);
        assert_eq!(r.marks[3], Mark::Missed);
        assert!(!r.passed());
    }

    #[test]
    fn an_extra_tap_fails_an_otherwise_perfect_run() {
        let mut r = RhythmRun::new(&[1.0, 1.0], 60.0, 80.0);
        assert_eq!(r.tap(0.0), Some(Mark::Good));
        assert_eq!(r.tap(30.0), None);
        assert_eq!(r.tap(1000.0), Some(Mark::Good));
        assert!(!r.passed());
    }

    #[test]
    fn arbitrary_onsets_with_pitch_filtering() {
        // Notes at 0, 500 and 1500 ms (a rest between): window is half the 500 ms gap.
        let mut r = RhythmRun::with_onsets(vec![0.0, 500.0, 1500.0], 2000.0, 150.0);
        assert_eq!(r.window_ms(), 250.0);
        assert_eq!(r.nearest(480.0, |i| i != 1), None, "the right note is filtered out by pitch");
        assert_eq!(r.nearest(480.0, |_| true), Some(1));
        assert_eq!(r.mark(1, 680.0), Mark::Late);
        assert!(r.near(1, 700.0) && !r.near(1, 800.0));
    }

    #[test]
    fn calibration_offset() {
        // Taps about 40 ms after each 600 ms click, one outlier, one early tap that wraps around.
        assert_eq!(median_offset(&[38.0, 642.0, 1240.0, 1999.0, 2440.0, 3590.0], 600.0), 40.0);
        assert_eq!(median_offset(&[], 600.0), 0.0);
    }
}
