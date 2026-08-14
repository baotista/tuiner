//! The Input Device / Input Channel picker: a pure reducer over move/confirm/back events,
//! independent of `cpal` and the terminal so it is unit-testable with no hardware.
//!
//! Two steps — Device then Channel — but the Channel step is skipped entirely for a
//! single-channel device (PRD: "skip the channel step entirely when the device exposes only
//! one, so a laptop microphone stays a single decision"). Reopening mid-session (the `i` key)
//! and a stale-or-vanished remembered device (future config work) both go through the same
//! `message` field, carrying the explanation shown above the list — one path, not two.

/// One Input Device as offered to the picker, independent of how it was enumerated.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceEntry {
    pub name: String,
    pub channels: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Device,
    Channel,
}

/// A confirmed Input Device and Input Channel, by index into the list the `Picker` was built
/// with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub device_idx: usize,
    pub channel_idx: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Still choosing.
    Continue,
    /// A Device and Channel have been confirmed.
    Selected(Selection),
    /// The player backed out of a picker that was allowed to be cancelled (reopened mid-session
    /// with an existing selection already running).
    Cancelled,
}

/// Drives the two-step choice. Not responsible for the empty-device-list case — a plain message
/// and a clean exit, per the PRD, is a `main`-level concern that never needs a `Picker` at all.
pub struct Picker {
    devices: Vec<DeviceEntry>,
    step: Step,
    device_idx: usize,
    channel_idx: usize,
    cancellable: bool,
    message: Option<String>,
}

impl Picker {
    /// `cancellable` should be false on the very first, mandatory pick (nothing to fall back to)
    /// and true when reopening mid-session or over a stale remembered selection. `message` is
    /// the explanation banner — `None` for a first-run pick, `Some(..)` for a reopen.
    pub fn new(devices: Vec<DeviceEntry>, cancellable: bool, message: Option<String>) -> Self {
        debug_assert!(
            !devices.is_empty(),
            "Picker requires at least one Input Device; the empty case is handled before one is built"
        );
        Self {
            devices,
            step: Step::Device,
            device_idx: 0,
            channel_idx: 0,
            cancellable,
            message,
        }
    }

    pub fn devices(&self) -> &[DeviceEntry] {
        &self.devices
    }

    pub fn step(&self) -> Step {
        self.step
    }

    pub fn device_idx(&self) -> usize {
        self.device_idx
    }

    pub fn channel_idx(&self) -> usize {
        self.channel_idx
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// The Device currently highlighted at the Device step, or chosen on entry to the Channel
    /// step — the one a live level meter should be streaming from.
    pub fn highlighted_device(&self) -> &DeviceEntry {
        &self.devices[self.device_idx]
    }

    fn current_len(&self) -> usize {
        match self.step {
            Step::Device => self.devices.len(),
            Step::Channel => self.highlighted_device().channels as usize,
        }
    }

    pub fn move_up(&mut self) {
        let len = self.current_len();
        let idx = self.index_mut();
        *idx = (*idx + len - 1) % len;
    }

    pub fn move_down(&mut self) {
        let len = self.current_len();
        let idx = self.index_mut();
        *idx = (*idx + 1) % len;
    }

    fn index_mut(&mut self) -> &mut usize {
        match self.step {
            Step::Device => &mut self.device_idx,
            Step::Channel => &mut self.channel_idx,
        }
    }

    /// Confirms the current step's highlighted entry. On the Device step, either finishes
    /// immediately (single-channel device) or advances to the Channel step; on the Channel step,
    /// finishes.
    pub fn confirm(&mut self) -> Outcome {
        match self.step {
            Step::Device => {
                if self.highlighted_device().channels <= 1 {
                    Outcome::Selected(Selection {
                        device_idx: self.device_idx,
                        channel_idx: 0,
                    })
                } else {
                    self.step = Step::Channel;
                    self.channel_idx = 0;
                    Outcome::Continue
                }
            }
            Step::Channel => Outcome::Selected(Selection {
                device_idx: self.device_idx,
                channel_idx: self.channel_idx,
            }),
        }
    }

    /// Steps back from Channel to Device, or cancels outright from the Device step if this
    /// picker was opened as cancellable. Ignored on the Device step when not cancellable — a
    /// mandatory first pick has nothing to fall back to.
    pub fn back_or_cancel(&mut self) -> Outcome {
        match self.step {
            Step::Channel => {
                self.step = Step::Device;
                Outcome::Continue
            }
            Step::Device if self.cancellable => Outcome::Cancelled,
            Step::Device => Outcome::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices(channels: &[u16]) -> Vec<DeviceEntry> {
        channels
            .iter()
            .enumerate()
            .map(|(i, &c)| DeviceEntry {
                name: format!("device-{i}"),
                channels: c,
            })
            .collect()
    }

    #[test]
    fn confirming_a_single_channel_device_skips_the_channel_step() {
        let mut p = Picker::new(devices(&[1, 2]), false, None);
        let outcome = p.confirm();
        assert_eq!(
            outcome,
            Outcome::Selected(Selection {
                device_idx: 0,
                channel_idx: 0
            })
        );
    }

    #[test]
    fn confirming_a_multi_channel_device_advances_to_the_channel_step() {
        let mut p = Picker::new(devices(&[1, 2]), false, None);
        p.move_down(); // highlight the 2-channel device
        let outcome = p.confirm();
        assert_eq!(outcome, Outcome::Continue);
        assert_eq!(p.step(), Step::Channel);
    }

    #[test]
    fn confirming_a_channel_completes_the_selection() {
        let mut p = Picker::new(devices(&[1, 2]), false, None);
        p.move_down();
        p.confirm();
        p.move_down(); // highlight channel 1
        let outcome = p.confirm();
        assert_eq!(
            outcome,
            Outcome::Selected(Selection {
                device_idx: 1,
                channel_idx: 1
            })
        );
    }

    #[test]
    fn move_down_wraps_at_the_end_of_the_device_list() {
        let mut p = Picker::new(devices(&[1, 1, 1]), false, None);
        p.move_down();
        p.move_down();
        p.move_down();
        assert_eq!(p.device_idx(), 0);
    }

    #[test]
    fn move_up_wraps_at_the_start_of_the_device_list() {
        let mut p = Picker::new(devices(&[1, 1, 1]), false, None);
        p.move_up();
        assert_eq!(p.device_idx(), 2);
    }

    #[test]
    fn channel_step_movement_is_bounded_by_that_devices_channel_count() {
        let mut p = Picker::new(devices(&[1, 4]), false, None);
        p.move_down(); // the 4-channel device
        p.confirm();
        for _ in 0..4 {
            p.move_down();
        }
        assert_eq!(p.channel_idx(), 0, "4 moves should wrap exactly once");
    }

    #[test]
    fn escape_on_the_channel_step_returns_to_the_device_step_without_finishing() {
        let mut p = Picker::new(devices(&[1, 2]), false, None);
        p.move_down();
        p.confirm();
        assert_eq!(p.step(), Step::Channel);
        let outcome = p.back_or_cancel();
        assert_eq!(outcome, Outcome::Continue);
        assert_eq!(p.step(), Step::Device);
    }

    #[test]
    fn escape_on_the_device_step_cancels_when_the_picker_is_cancellable() {
        let mut p = Picker::new(devices(&[1]), true, None);
        assert_eq!(p.back_or_cancel(), Outcome::Cancelled);
    }

    #[test]
    fn escape_on_the_device_step_is_ignored_when_the_pick_is_mandatory() {
        let mut p = Picker::new(devices(&[1]), false, None);
        assert_eq!(p.back_or_cancel(), Outcome::Continue);
        assert_eq!(p.step(), Step::Device);
    }

    #[test]
    fn a_reopen_message_is_carried_for_the_ui_to_show() {
        let p = Picker::new(devices(&[1]), true, Some("Input Device disappeared".into()));
        assert_eq!(p.message(), Some("Input Device disappeared"));
    }

    #[test]
    fn no_message_on_a_first_run_pick() {
        let p = Picker::new(devices(&[1]), false, None);
        assert_eq!(p.message(), None);
    }
}
