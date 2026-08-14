//! The Deviation Trail: recent history of Deviation against time, so the player can see whether
//! a String is converging, overshooting, or drifting back out after being turned — information
//! no instantaneous readout can convey.
//!
//! A fixed-capacity ring buffer, not a growable log: it scrolls by evicting the oldest sample as
//! a new one arrives, so it never reallocates once its capacity is reserved and never grows
//! without bound regardless of session length.
//!
//! A `Gap` is a real, preserved fact — a Silent or Unpitched hop — not a hole to be smoothed over.
//! Interpolating through it would draw a line across a stretch where no Pitch was ever measured.

use std::collections::VecDeque;

/// One hop's contribution to the Trail: a measured Deviation, in cents, or a gap where nothing
/// trustworthy was sounding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrailSample {
    Deviation(f32),
    Gap,
}

/// A ring buffer of the most recent `capacity` samples, oldest first.
pub struct Trail {
    samples: VecDeque<TrailSample>,
    capacity: usize,
}

impl Trail {
    /// `capacity` is clamped to at least 1 — a zero-capacity Trail has no sensible reading to
    /// evict from.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Pushes one new sample, scrolling out the oldest once at capacity. `samples` was
    /// constructed with `with_capacity(capacity)` and never holds more than `capacity` entries,
    /// so this never triggers a reallocation.
    pub fn push(&mut self, sample: TrailSample) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// The currently held samples, oldest first — shorter than `capacity` until the Trail has
    /// been running long enough to fill up.
    pub fn samples(&self) -> impl Iterator<Item = &TrailSample> {
        self.samples.iter()
    }

    /// A full `capacity`-long snapshot, left-padded with `Gap` for any history not yet
    /// accumulated. Lets a renderer always lay Deviation out against a fixed timeline: early in
    /// a session, sparse real history stays anchored to the right edge rather than being
    /// stretched to fill a display meant to show 10 seconds' worth.
    pub fn padded_samples(&self) -> Vec<TrailSample> {
        let padding = self.capacity - self.samples.len();
        std::iter::repeat_n(TrailSample::Gap, padding)
            .chain(self.samples.iter().copied())
            .collect()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushing_past_capacity_scrolls_out_the_oldest_sample() {
        let mut trail = Trail::new(3);
        trail.push(TrailSample::Deviation(1.0));
        trail.push(TrailSample::Deviation(2.0));
        trail.push(TrailSample::Deviation(3.0));
        trail.push(TrailSample::Deviation(4.0)); // should evict the 1.0 sample

        let samples: Vec<TrailSample> = trail.samples().copied().collect();
        assert_eq!(
            samples,
            vec![
                TrailSample::Deviation(2.0),
                TrailSample::Deviation(3.0),
                TrailSample::Deviation(4.0),
            ]
        );
    }

    #[test]
    fn capacity_is_never_exceeded_no_matter_how_many_pushes() {
        let mut trail = Trail::new(5);
        for i in 0..1000 {
            trail.push(TrailSample::Deviation(i as f32));
        }
        assert_eq!(trail.samples().count(), 5);
    }

    #[test]
    fn gap_and_value_samples_are_preserved_distinctly_not_merged() {
        let mut trail = Trail::new(10);
        trail.push(TrailSample::Deviation(-2.0));
        trail.push(TrailSample::Gap);
        trail.push(TrailSample::Gap);
        trail.push(TrailSample::Deviation(3.0));

        let samples: Vec<TrailSample> = trail.samples().copied().collect();
        assert_eq!(
            samples,
            vec![
                TrailSample::Deviation(-2.0),
                TrailSample::Gap,
                TrailSample::Gap,
                TrailSample::Deviation(3.0),
            ]
        );
    }

    #[test]
    fn zero_capacity_is_clamped_to_one_rather_than_panicking() {
        let mut trail = Trail::new(0);
        trail.push(TrailSample::Deviation(1.0));
        trail.push(TrailSample::Deviation(2.0));
        assert_eq!(trail.capacity(), 1);
        assert_eq!(
            trail.samples().copied().collect::<Vec<_>>(),
            vec![TrailSample::Deviation(2.0)]
        );
    }

    #[test]
    fn a_fresh_trail_holds_nothing() {
        let trail = Trail::new(10);
        assert_eq!(trail.samples().count(), 0);
    }

    #[test]
    fn padded_samples_left_pads_with_gap_until_the_trail_fills_up() {
        let mut trail = Trail::new(5);
        trail.push(TrailSample::Deviation(1.0));
        trail.push(TrailSample::Deviation(2.0));

        let padded = trail.padded_samples();
        assert_eq!(
            padded,
            vec![
                TrailSample::Gap,
                TrailSample::Gap,
                TrailSample::Gap,
                TrailSample::Deviation(1.0),
                TrailSample::Deviation(2.0),
            ]
        );
    }

    #[test]
    fn padded_samples_needs_no_padding_once_the_trail_is_full() {
        let mut trail = Trail::new(3);
        for i in 0..3 {
            trail.push(TrailSample::Deviation(i as f32));
        }
        assert_eq!(
            trail.padded_samples(),
            trail.samples().copied().collect::<Vec<_>>()
        );
    }
}
