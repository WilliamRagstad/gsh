//! Version-1 semantic input protocol for WebRTC data channels.
//!
//! Reliable transitions and replaceable state are parsed separately. This
//! module performs bounded JSON parsing and per-session sequence/epoch checks;
//! platform key mapping and application dispatch remain above this boundary.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

pub const MAX_CONTROL_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_REALTIME_MESSAGE_BYTES: usize = 4096;
const MAX_KEY_CODE_BYTES: usize = 128;
const MAX_HELD_KEYS: usize = 256;
const MAX_SCROLL_DELTA: f64 = 1_000_000.0;
const MAX_RELATIVE_DELTA: u64 = 100_000;

#[derive(Debug, Error)]
pub enum InputProtocolError {
    #[error("input message exceeds its channel limit")]
    Oversized,
    #[error("invalid input JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("input field is outside its allowed range")]
    InvalidField,
    #[error("per-session pressed-key limit exceeded")]
    TooManyHeldKeys,
}

/// Normalized semantic input; browser-specific names are kept until the
/// platform/input adapter maps them to a native key representation.
#[derive(Debug, Clone, PartialEq)]
pub enum GshInputEvent {
    Key {
        code: String,
        down: bool,
    },
    Button {
        button: u8,
        down: bool,
        x: f32,
        y: f32,
    },
    Pointer {
        x: f32,
        y: f32,
    },
    RelativeMotion {
        dx: i64,
        dy: i64,
    },
    Scroll {
        dx: f64,
        dy: f64,
        mode: u8,
    },
    ReleaseAll,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
enum ControlMessage {
    Key {
        seq: u64,
        epoch: u64,
        code: String,
        down: bool,
    },
    Button {
        seq: u64,
        epoch: u64,
        button: u8,
        down: bool,
        x: f32,
        y: f32,
    },
    Scroll {
        seq: u64,
        epoch: u64,
        dx: f64,
        dy: f64,
        mode: u8,
    },
    ReleaseAll {
        seq: u64,
        epoch: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
enum RealtimeMessage {
    Pointer {
        seq: u64,
        epoch: u64,
        x: f32,
        y: f32,
    },
    RelativeMotion {
        seq: u64,
        epoch: u64,
        dx_total: i64,
        dy_total: i64,
    },
}

/// State is owned per peer/session. It rejects stale/reordered snapshots,
/// tracks held transitions, and converts cumulative motion into the missing
/// delta since the previous accepted snapshot.
#[derive(Debug, Default)]
pub struct GshInputRouter {
    epoch: Option<u64>,
    last_control_seq: u64,
    last_realtime_seq: u64,
    last_dx_total: i64,
    last_dy_total: i64,
    keys_down: HashMap<String, ()>,
    buttons_down: HashSet<u8>,
}

impl GshInputRouter {
    pub fn handle_control(
        &mut self,
        bytes: &[u8],
    ) -> Result<Vec<GshInputEvent>, InputProtocolError> {
        if bytes.len() > MAX_CONTROL_MESSAGE_BYTES {
            return Err(InputProtocolError::Oversized);
        }
        let message: ControlMessage = serde_json::from_slice(bytes)?;
        match &message {
            ControlMessage::Key { code, .. } => {
                if code.is_empty() || code.len() > MAX_KEY_CODE_BYTES {
                    return Err(InputProtocolError::InvalidField);
                }
            }
            ControlMessage::Button { button, x, y, .. } => {
                validate_position(*x, *y)?;
                if *button > 2 {
                    return Err(InputProtocolError::InvalidField);
                }
            }
            ControlMessage::Scroll { dx, dy, mode, .. } => {
                if !dx.is_finite()
                    || !dy.is_finite()
                    || dx.abs() > MAX_SCROLL_DELTA
                    || dy.abs() > MAX_SCROLL_DELTA
                    || *mode > 2
                {
                    return Err(InputProtocolError::InvalidField);
                }
            }
            ControlMessage::ReleaseAll { .. } => {}
        }
        let (seq, epoch) = match &message {
            ControlMessage::Key { seq, epoch, .. }
            | ControlMessage::Button { seq, epoch, .. }
            | ControlMessage::Scroll { seq, epoch, .. }
            | ControlMessage::ReleaseAll { seq, epoch } => (*seq, *epoch),
        };
        let Some(mut events) = self.transition_epoch(epoch) else {
            return Ok(Vec::new());
        };
        if seq <= self.last_control_seq {
            return Ok(events);
        }
        match message {
            ControlMessage::Key { code, down, .. } => {
                if down {
                    if !self.keys_down.contains_key(&code) && self.keys_down.len() >= MAX_HELD_KEYS
                    {
                        return Err(InputProtocolError::TooManyHeldKeys);
                    }
                    if self.keys_down.insert(code.clone(), ()).is_none() {
                        events.push(GshInputEvent::Key { code, down });
                    }
                } else if self.keys_down.remove(&code).is_some() {
                    events.push(GshInputEvent::Key { code, down });
                }
            }
            ControlMessage::Button {
                button, down, x, y, ..
            } => {
                let changed = if down {
                    self.buttons_down.insert(button)
                } else {
                    self.buttons_down.remove(&button)
                };
                if changed {
                    events.push(GshInputEvent::Button { button, down, x, y });
                }
            }
            ControlMessage::Scroll { dx, dy, mode, .. } => {
                events.push(GshInputEvent::Scroll { dx, dy, mode });
            }
            ControlMessage::ReleaseAll { .. } => events.extend(self.release_all()),
        }
        self.last_control_seq = seq;
        Ok(events)
    }

    pub fn handle_realtime(
        &mut self,
        bytes: &[u8],
    ) -> Result<Vec<GshInputEvent>, InputProtocolError> {
        if bytes.len() > MAX_REALTIME_MESSAGE_BYTES {
            return Err(InputProtocolError::Oversized);
        }
        let message: RealtimeMessage = serde_json::from_slice(bytes)?;
        if let RealtimeMessage::Pointer { x, y, .. } = &message {
            validate_position(*x, *y)?;
        }
        let (seq, epoch) = match &message {
            RealtimeMessage::Pointer { seq, epoch, .. }
            | RealtimeMessage::RelativeMotion { seq, epoch, .. } => (*seq, *epoch),
        };
        let Some(mut events) = self.transition_epoch(epoch) else {
            return Ok(Vec::new());
        };
        if seq <= self.last_realtime_seq {
            return Ok(events);
        }
        self.last_realtime_seq = seq;

        match message {
            RealtimeMessage::Pointer { x, y, .. } => {
                events.push(GshInputEvent::Pointer { x, y });
            }
            RealtimeMessage::RelativeMotion {
                dx_total, dy_total, ..
            } => {
                let delta = dx_total
                    .checked_sub(self.last_dx_total)
                    .zip(dy_total.checked_sub(self.last_dy_total));
                self.last_dx_total = dx_total;
                self.last_dy_total = dy_total;
                if let Some((dx, dy)) = delta {
                    if dx.unsigned_abs() <= MAX_RELATIVE_DELTA
                        && dy.unsigned_abs() <= MAX_RELATIVE_DELTA
                        && (dx != 0 || dy != 0)
                    {
                        events.push(GshInputEvent::RelativeMotion { dx, dy });
                    }
                    // An implausibly large delta resets the baseline but is not
                    // dispatched, preventing a wrap/reset from teleporting input.
                }
            }
        }
        Ok(events)
    }

    /// Release all held input when the session loses focus or disconnects.
    pub fn release_all(&mut self) -> Vec<GshInputEvent> {
        let mut events = self
            .keys_down
            .drain()
            .map(|(code, ())| GshInputEvent::Key { code, down: false })
            .collect::<Vec<_>>();
        events.extend(
            self.buttons_down
                .drain()
                .map(|button| GshInputEvent::Button {
                    button,
                    down: false,
                    x: 0.0,
                    y: 0.0,
                }),
        );
        events
    }

    fn transition_epoch(&mut self, epoch: u64) -> Option<Vec<GshInputEvent>> {
        match self.epoch {
            Some(current) if epoch < current => None,
            Some(current) if epoch == current => Some(Vec::new()),
            _ => {
                let events = self.release_all();
                self.epoch = Some(epoch);
                self.last_control_seq = 0;
                self.last_realtime_seq = 0;
                self.last_dx_total = 0;
                self.last_dy_total = 0;
                Some(events)
            }
        }
    }
}

fn validate_position(x: f32, y: f32) -> Result<(), InputProtocolError> {
    if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return Err(InputProtocolError::InvalidField);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_transitions_are_reliable_state_changes_with_release_all() {
        let mut router = GshInputRouter::default();
        let down = router
            .handle_control(br#"{"t":"key","seq":1,"epoch":7,"code":"KeyW","down":true}"#)
            .unwrap();
        assert_eq!(
            down,
            vec![GshInputEvent::Key {
                code: "KeyW".into(),
                down: true
            }]
        );
        assert!(router
            .handle_control(br#"{"t":"key","seq":1,"epoch":7,"code":"KeyW","down":true}"#)
            .unwrap()
            .is_empty());
        let released = router.release_all();
        assert_eq!(
            released,
            vec![GshInputEvent::Key {
                code: "KeyW".into(),
                down: false
            }]
        );
    }

    #[test]
    fn cumulative_relative_motion_recovers_missing_snapshots_and_drops_reordering() {
        let mut router = GshInputRouter::default();
        assert_eq!(
            router
                .handle_realtime(
                    br#"{"t":"relative_motion","seq":1,"epoch":9,"dx_total":5,"dy_total":2}"#
                )
                .unwrap(),
            vec![GshInputEvent::RelativeMotion { dx: 5, dy: 2 }]
        );
        // seq=2 / total=(8, 5) is intentionally lost. The next state recovers
        // the combined delta from the last applied cumulative total.
        assert_eq!(
            router
                .handle_realtime(
                    br#"{"t":"relative_motion","seq":3,"epoch":9,"dx_total":12,"dy_total":7}"#
                )
                .unwrap(),
            vec![GshInputEvent::RelativeMotion { dx: 7, dy: 5 }]
        );
        assert!(router
            .handle_realtime(
                br#"{"t":"relative_motion","seq":2,"epoch":9,"dx_total":8,"dy_total":5}"#
            )
            .unwrap()
            .is_empty());
    }

    #[test]
    fn epoch_change_releases_held_keys_and_rejects_invalid_coordinates() {
        let mut router = GshInputRouter::default();
        router
            .handle_control(br#"{"t":"key","seq":1,"epoch":11,"code":"KeyA","down":true}"#)
            .unwrap();
        assert_eq!(
            router
                .handle_realtime(br#"{"t":"pointer","seq":1,"epoch":12,"x":0.5,"y":0.25}"#)
                .unwrap(),
            vec![
                GshInputEvent::Key {
                    code: "KeyA".into(),
                    down: false
                },
                GshInputEvent::Pointer { x: 0.5, y: 0.25 },
            ]
        );
        assert!(matches!(
            router.handle_realtime(br#"{"t":"pointer","seq":2,"epoch":12,"x":2.0,"y":0.0}"#),
            Err(InputProtocolError::InvalidField)
        ));
    }

    #[test]
    fn oversized_control_payload_is_rejected_before_json_parsing() {
        let mut router = GshInputRouter::default();
        let payload = vec![b' '; MAX_CONTROL_MESSAGE_BYTES + 1];
        assert!(matches!(
            router.handle_control(&payload),
            Err(InputProtocolError::Oversized)
        ));
    }

    #[test]
    fn older_input_epoch_cannot_reset_current_state() {
        let mut router = GshInputRouter::default();
        router
            .handle_control(br#"{"t":"key","seq":1,"epoch":20,"code":"KeyW","down":true}"#)
            .unwrap();
        assert!(router
            .handle_control(br#"{"t":"release_all","seq":99,"epoch":19}"#)
            .unwrap()
            .is_empty());
        assert_eq!(
            router.release_all(),
            vec![GshInputEvent::Key {
                code: "KeyW".into(),
                down: false
            }]
        );
    }

    #[test]
    fn invalid_control_payload_cannot_poison_epoch_or_sequence() {
        let mut router = GshInputRouter::default();
        router
            .handle_control(br#"{"t":"key","seq":1,"epoch":7,"code":"KeyW","down":true}"#)
            .unwrap();

        assert!(matches!(
            router.handle_control(
                br#"{"t":"button","seq":100,"epoch":18446744073709551615,"button":0,"down":true,"x":2.0,"y":0.5}"#
            ),
            Err(InputProtocolError::InvalidField)
        ));
        assert_eq!(
            router
                .handle_control(br#"{"t":"key","seq":2,"epoch":7,"code":"KeyW","down":false}"#)
                .unwrap(),
            vec![GshInputEvent::Key {
                code: "KeyW".into(),
                down: false
            }]
        );
    }

    #[test]
    fn invalid_pointer_cannot_poison_epoch_or_sequence() {
        let mut router = GshInputRouter::default();
        router
            .handle_realtime(br#"{"t":"pointer","seq":1,"epoch":7,"x":0.5,"y":0.5}"#)
            .unwrap();

        assert!(matches!(
            router.handle_realtime(
                br#"{"t":"pointer","seq":100,"epoch":18446744073709551615,"x":2.0,"y":0.5}"#
            ),
            Err(InputProtocolError::InvalidField)
        ));
        assert_eq!(
            router
                .handle_realtime(br#"{"t":"pointer","seq":2,"epoch":7,"x":0.6,"y":0.5}"#)
                .unwrap(),
            vec![GshInputEvent::Pointer { x: 0.6, y: 0.5 }]
        );
    }

    #[test]
    fn held_key_state_is_bounded_per_session() {
        let mut router = GshInputRouter::default();
        for seq in 1..=MAX_HELD_KEYS {
            let message =
                format!(r#"{{"t":"key","seq":{seq},"epoch":1,"code":"Code{seq}","down":true}}"#);
            router.handle_control(message.as_bytes()).unwrap();
        }
        let seq = MAX_HELD_KEYS + 1;
        let message =
            format!(r#"{{"t":"key","seq":{seq},"epoch":1,"code":"Code{seq}","down":true}}"#);
        assert!(matches!(
            router.handle_control(message.as_bytes()),
            Err(InputProtocolError::TooManyHeldKeys)
        ));
    }
}
