use std::collections::{HashMap, HashSet};

use crate::model::{
    ControlId, InputEvent, InputPhase, InputSource, TrustedHardwareSource, VirtualInputEvent,
};

#[derive(Debug, Default)]
struct SourceState {
    last_sequence: Option<u64>,
    ready: bool,
    pressed: HashSet<ControlId>,
    suppressed_until_release: HashSet<ControlId>,
}

#[derive(Debug, Default)]
pub struct InputCoordinator {
    sources: HashMap<InputSource, SourceState>,
    contributions: HashMap<ControlId, usize>,
    next_app_sequence: u64,
}

#[allow(dead_code)]
impl InputCoordinator {
    pub fn handle_virtual(&mut self, event: VirtualInputEvent) -> Result<Vec<InputEvent>, String> {
        self.sources
            .entry(InputSource::VirtualUi)
            .or_insert_with(|| SourceState {
                ready: true,
                ..SourceState::default()
            });
        self.handle(
            InputSource::VirtualUi,
            event.sequence,
            event.monotonic_ms,
            event.control,
            event.phase,
        )
    }

    pub fn register_hardware(&mut self, source: TrustedHardwareSource) {
        self.sources
            .insert(InputSource::Hardware(source), SourceState::default());
    }

    pub fn synchronize_hardware(
        &mut self,
        source: TrustedHardwareSource,
        sequence: u32,
        pressed: impl IntoIterator<Item = ControlId>,
    ) -> Result<(), String> {
        let state = self
            .sources
            .get_mut(&InputSource::Hardware(source))
            .ok_or_else(|| "unknown_hardware_source".to_string())?;
        state.last_sequence = Some(u64::from(sequence));
        state.ready = true;
        state.pressed.clear();
        state.suppressed_until_release = pressed.into_iter().collect();
        Ok(())
    }

    pub fn handle_hardware(
        &mut self,
        source: TrustedHardwareSource,
        sequence: u32,
        monotonic_ms: f64,
        control: ControlId,
        phase: InputPhase,
    ) -> Result<Vec<InputEvent>, String> {
        self.handle(
            InputSource::Hardware(source),
            u64::from(sequence),
            monotonic_ms,
            control,
            phase,
        )
    }

    pub fn observe_hardware(
        &mut self,
        source: TrustedHardwareSource,
        sequence: u32,
    ) -> Result<(), String> {
        let state = self
            .sources
            .get_mut(&InputSource::Hardware(source))
            .ok_or_else(|| "unknown_hardware_source".to_string())?;
        if !state.ready {
            return Ok(());
        }
        validate_sequence(state, u64::from(sequence))
    }

    pub fn desynchronize_hardware(&mut self, source: TrustedHardwareSource) -> Vec<InputEvent> {
        self.release_source(InputSource::Hardware(source), true)
    }

    pub fn disconnect_hardware(&mut self, source: TrustedHardwareSource) -> Vec<InputEvent> {
        self.release_source(InputSource::Hardware(source), false)
    }

    pub fn clear_all(&mut self) {
        self.sources.clear();
        self.contributions.clear();
    }

    fn handle(
        &mut self,
        source: InputSource,
        producer_sequence: u64,
        monotonic_ms: f64,
        control: ControlId,
        phase: InputPhase,
    ) -> Result<Vec<InputEvent>, String> {
        let state = self
            .sources
            .get_mut(&source)
            .ok_or_else(|| "unknown_input_source".to_string())?;
        if !state.ready {
            return Err("input_source_not_ready".into());
        }
        validate_sequence(state, producer_sequence)?;

        validate_control_phase(&control, &phase)?;
        if state.suppressed_until_release.contains(&control) {
            if is_release(&control, &phase) {
                state.suppressed_until_release.remove(&control);
            }
            return Ok(Vec::new());
        }

        if matches!(phase, InputPhase::Rotated { .. }) {
            return Ok(vec![self.normalized(source, control, phase, monotonic_ms)]);
        }

        if is_press(&control, &phase) {
            if !state.pressed.insert(control.clone()) {
                return Ok(Vec::new());
            }
            let count = self.contributions.entry(control.clone()).or_default();
            *count += 1;
            if *count > 1 {
                return Ok(Vec::new());
            }
        } else if !state.pressed.remove(&control) {
            return Ok(Vec::new());
        } else {
            let count = self
                .contributions
                .get_mut(&control)
                .expect("a pressed source must have a contribution");
            *count -= 1;
            if *count > 0 {
                return Ok(Vec::new());
            }
            self.contributions.remove(&control);
        }

        Ok(vec![self.normalized(source, control, phase, monotonic_ms)])
    }

    fn release_source(&mut self, source: InputSource, retain_source: bool) -> Vec<InputEvent> {
        let Some(mut state) = self.sources.remove(&source) else {
            return Vec::new();
        };
        let mut releases = Vec::new();
        for control in state.pressed.drain() {
            let Some(count) = self.contributions.get_mut(&control) else {
                continue;
            };
            *count -= 1;
            if *count == 0 {
                self.contributions.remove(&control);
                let phase = match control {
                    ControlId::Joystick { .. } => InputPhase::Centered,
                    _ => InputPhase::Released,
                };
                releases.push(self.normalized(source, control, phase, 0.0));
            }
        }
        if retain_source {
            state.ready = false;
            state.suppressed_until_release.clear();
            self.sources.insert(source, state);
        }
        releases
    }

    fn normalized(
        &mut self,
        source: InputSource,
        control: ControlId,
        phase: InputPhase,
        monotonic_ms: f64,
    ) -> InputEvent {
        self.next_app_sequence = self
            .next_app_sequence
            .checked_add(1)
            .expect("App input sequence exhausted");
        InputEvent {
            source,
            control,
            phase,
            sequence: self.next_app_sequence,
            monotonic_ms,
        }
    }
}

fn validate_sequence(state: &mut SourceState, producer_sequence: u64) -> Result<(), String> {
    let expected = state.last_sequence.map_or(1, |last| last.saturating_add(1));
    if producer_sequence != expected {
        return Err(
            if state
                .last_sequence
                .is_some_and(|last| producer_sequence <= last)
            {
                "duplicate_or_out_of_order_input".into()
            } else {
                "input_sequence_gap".into()
            },
        );
    }
    state.last_sequence = Some(producer_sequence);
    Ok(())
}

fn validate_control_phase(control: &ControlId, phase: &InputPhase) -> Result<(), String> {
    match (control, phase) {
        (ControlId::EncoderRotate, InputPhase::Rotated { steps })
            if *steps != 0 && steps.unsigned_abs() <= 32 =>
        {
            Ok(())
        }
        (ControlId::EncoderRotate, _) | (_, InputPhase::Rotated { .. }) => {
            Err("invalid_encoder_rotation".into())
        }
        (ControlId::Joystick { .. }, InputPhase::Entered | InputPhase::Centered) => Ok(()),
        (ControlId::Joystick { .. }, _) => Err("invalid_joystick_phase".into()),
        (_, InputPhase::Pressed | InputPhase::Released) => Ok(()),
        (_, InputPhase::Entered | InputPhase::Centered) => Err("invalid_edge_phase".into()),
    }
}

fn is_press(control: &ControlId, phase: &InputPhase) -> bool {
    matches!(
        (control, phase),
        (ControlId::Joystick { .. }, InputPhase::Entered) | (_, InputPhase::Pressed)
    )
}

fn is_release(control: &ControlId, phase: &InputPhase) -> bool {
    matches!(
        (control, phase),
        (ControlId::Joystick { .. }, InputPhase::Centered) | (_, InputPhase::Released)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HardwareTransport;

    fn hardware(id: u64, generation: u64) -> TrustedHardwareSource {
        TrustedHardwareSource {
            id,
            transport: HardwareTransport::Fake,
            generation,
        }
    }

    #[test]
    fn two_sources_keep_generation_sequence_and_pressed_state_independent() {
        let mut coordinator = InputCoordinator::default();
        let first = hardware(1, 10);
        let second = hardware(2, 20);
        let control = ControlId::Agent { slot: 0 };
        coordinator.register_hardware(first);
        coordinator.register_hardware(second);
        coordinator.synchronize_hardware(first, 2, []).unwrap();
        coordinator.synchronize_hardware(second, 40, []).unwrap();

        assert_eq!(
            coordinator
                .handle_hardware(first, 3, 1.0, control.clone(), InputPhase::Pressed)
                .unwrap()
                .len(),
            1
        );
        assert!(coordinator
            .handle_hardware(second, 41, 2.0, control.clone(), InputPhase::Pressed)
            .unwrap()
            .is_empty());
        assert!(coordinator.disconnect_hardware(first).is_empty());
        let releases = coordinator.disconnect_hardware(second);
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].phase, InputPhase::Released);
    }

    #[test]
    fn virtual_and_hardware_sources_share_one_app_sequence() {
        let mut coordinator = InputCoordinator::default();
        let hardware = hardware(1, 1);
        coordinator.register_hardware(hardware);
        coordinator.synchronize_hardware(hardware, 2, []).unwrap();

        let virtual_events = coordinator
            .handle_virtual(VirtualInputEvent {
                source: crate::model::VirtualInputSource::VirtualUi,
                control: ControlId::EncoderRotate,
                phase: InputPhase::Rotated { steps: 1 },
                sequence: 1,
                monotonic_ms: 1.0,
            })
            .unwrap();
        let hardware_events = coordinator
            .handle_hardware(
                hardware,
                3,
                2.0,
                ControlId::EncoderRotate,
                InputPhase::Rotated { steps: -1 },
            )
            .unwrap();

        assert_eq!(virtual_events[0].sequence, 1);
        assert_eq!(hardware_events[0].sequence, 2);
    }

    #[test]
    fn stale_generation_and_sequence_anomalies_fail_closed() {
        let mut coordinator = InputCoordinator::default();
        let current = hardware(1, 2);
        let stale = hardware(1, 1);
        coordinator.register_hardware(current);
        coordinator.synchronize_hardware(current, 9, []).unwrap();

        assert!(coordinator
            .handle_hardware(
                stale,
                10,
                1.0,
                ControlId::Agent { slot: 0 },
                InputPhase::Pressed
            )
            .is_err());
        assert!(coordinator
            .handle_hardware(
                current,
                9,
                1.0,
                ControlId::Agent { slot: 0 },
                InputPhase::Pressed
            )
            .is_err());
        assert!(coordinator
            .handle_hardware(
                current,
                11,
                1.0,
                ControlId::Agent { slot: 0 },
                InputPhase::Pressed
            )
            .is_err());
        assert_eq!(
            coordinator
                .handle_hardware(
                    current,
                    10,
                    1.0,
                    ControlId::Agent { slot: 0 },
                    InputPhase::Pressed
                )
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn snapshot_pressed_controls_are_suppressed_until_release() {
        let mut coordinator = InputCoordinator::default();
        let source = hardware(3, 1);
        let control = ControlId::Agent { slot: 0 };
        coordinator.register_hardware(source);
        coordinator
            .synchronize_hardware(source, 2, [control.clone()])
            .unwrap();

        assert!(coordinator
            .handle_hardware(source, 3, 0.0, control, InputPhase::Released)
            .unwrap()
            .is_empty());
    }
}
