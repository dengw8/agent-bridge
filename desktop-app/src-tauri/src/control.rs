use std::collections::HashSet;

use crate::model::{
    AgentSlot, Binding, ControlId, InputEvent, InputPhase, Intent, Layout, OFFICIAL_KEYCAP_IDS,
};

#[derive(Debug, Default)]
pub struct ControlEngine {
    pressed: HashSet<ControlId>,
    last_sequence: Option<u64>,
}

impl ControlEngine {
    pub fn handle(
        &mut self,
        event: InputEvent,
        layout: &Layout,
        agents: &[AgentSlot],
    ) -> Result<Vec<Intent>, String> {
        if self
            .last_sequence
            .is_some_and(|last| event.sequence <= last)
        {
            return Err("Input sequence must increase monotonically.".into());
        }
        self.last_sequence = Some(event.sequence);
        match (&event.control, &event.phase) {
            (ControlId::EncoderRotate, InputPhase::Rotated { steps }) => {
                if *steps == 0 || steps.unsigned_abs() > 32 {
                    return Err("Encoder rotation must contain 1 to 32 steps.".into());
                }
                return Ok(vec![Intent::EncoderRotate {
                    mode: layout.encoder_mode,
                    steps: *steps,
                }]);
            }
            (_, InputPhase::Rotated { .. }) => {
                return Err("Only the encoder can emit rotated input.".into())
            }
            (ControlId::Joystick { .. }, InputPhase::Entered) | (_, InputPhase::Pressed) => {
                if !self.pressed.insert(event.control.clone()) {
                    return Ok(Vec::new());
                }
            }
            (ControlId::Joystick { .. }, InputPhase::Centered) | (_, InputPhase::Released) => {
                if !self.pressed.remove(&event.control) {
                    return Ok(Vec::new());
                }
            }
            (_, InputPhase::Entered | InputPhase::Centered) => {
                return Err("Only joystick controls can enter or center.".into())
            }
        }
        self.intent_for_control(&event.control, &event.phase, layout, agents)
    }

    pub fn reset(&mut self, layout: &Layout, agents: &[AgentSlot]) -> Vec<Intent> {
        let controls: Vec<_> = self.pressed.drain().collect();
        controls
            .into_iter()
            .filter_map(|control| {
                let phase = match control {
                    ControlId::Joystick { .. } => InputPhase::Centered,
                    _ => InputPhase::Released,
                };
                self.intent_for_control(&control, &phase, layout, agents)
                    .ok()
                    .and_then(|mut intents| intents.pop())
            })
            .collect()
    }

    pub fn clear_pressed(&mut self) {
        self.pressed.clear();
    }

    fn intent_for_control(
        &self,
        control: &ControlId,
        phase: &InputPhase,
        layout: &Layout,
        agents: &[AgentSlot],
    ) -> Result<Vec<Intent>, String> {
        let pressed = matches!(phase, InputPhase::Pressed | InputPhase::Entered);
        let intent = match control {
            ControlId::Agent { slot } => {
                if *slot > 5 {
                    return Err(format!("Invalid Agent slot: {slot}"));
                }
                Intent::AgentEdge {
                    slot: *slot,
                    thread_key: agents
                        .iter()
                        .find(|agent| agent.slot == *slot)
                        .and_then(|agent| agent.thread_key.clone()),
                    pressed,
                }
            }
            ControlId::Command { slot } => {
                let binding = layout
                    .command_slots
                    .get(slot)
                    .cloned()
                    .ok_or_else(|| format!("Missing layout binding for {}", slot.as_str()))?;
                if matches!(binding, Binding::None) {
                    return Ok(Vec::new());
                }
                Intent::ExecuteBinding {
                    slot: Some(*slot),
                    binding,
                    pressed,
                }
            }
            ControlId::Keycap { id } => {
                if !OFFICIAL_KEYCAP_IDS.contains(&id.as_str()) {
                    return Err(format!("Unsupported keycap: {id}"));
                }
                Intent::ExecuteBinding {
                    slot: None,
                    binding: Binding::Keycap { id: id.clone() },
                    pressed,
                }
            }
            ControlId::Joystick { direction } => {
                let binding = layout.joystick.get(direction).cloned().ok_or_else(|| {
                    format!("Missing joystick binding for {}", direction.as_str())
                })?;
                if matches!(binding, Binding::None) {
                    return Ok(Vec::new());
                }
                if matches!(binding, Binding::Keycap { .. }) {
                    return Err("Joystick mappings must be a command or skill.".into());
                }
                Intent::JoystickEdge {
                    direction: *direction,
                    active: pressed,
                    binding,
                }
            }
            ControlId::EncoderPress => Intent::EncoderEdge {
                mode: layout.encoder_mode,
                pressed,
            },
            ControlId::EncoderRotate => unreachable!("rotation is handled before edge state"),
        };
        Ok(vec![intent])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CommandSlot, Direction, InputSource};

    fn event(control: ControlId, phase: InputPhase, sequence: u64) -> InputEvent {
        InputEvent {
            source: InputSource::VirtualUi,
            control,
            phase,
            sequence,
            monotonic_ms: sequence as f64,
        }
    }

    #[test]
    fn command_press_and_release_keep_phase_and_app_binding() {
        let mut engine = ControlEngine::default();
        let mut layout = Layout::official_defaults();
        layout
            .command_slots
            .insert(CommandSlot::Act06, Binding::Keycap { id: "MIC".into() });
        let agents: Vec<_> = (0..6).map(AgentSlot::empty).collect();

        let down = engine
            .handle(
                event(
                    ControlId::Command {
                        slot: CommandSlot::Act06,
                    },
                    InputPhase::Pressed,
                    1,
                ),
                &layout,
                &agents,
            )
            .unwrap();
        let up = engine
            .handle(
                event(
                    ControlId::Command {
                        slot: CommandSlot::Act06,
                    },
                    InputPhase::Released,
                    2,
                ),
                &layout,
                &agents,
            )
            .unwrap();
        assert_eq!(
            down[0],
            Intent::ExecuteBinding {
                slot: Some(CommandSlot::Act06),
                binding: Binding::Keycap { id: "MIC".into() },
                pressed: true,
            }
        );
        assert!(matches!(
            up[0],
            Intent::ExecuteBinding { pressed: false, .. }
        ));
    }

    #[test]
    fn duplicate_edges_and_out_of_order_sequences_fail_closed() {
        let mut engine = ControlEngine::default();
        let layout = Layout::official_defaults();
        let agents: Vec<_> = (0..6).map(AgentSlot::empty).collect();
        let control = ControlId::Agent { slot: 0 };
        assert_eq!(
            engine
                .handle(
                    event(control.clone(), InputPhase::Pressed, 1),
                    &layout,
                    &agents
                )
                .unwrap()
                .len(),
            1
        );
        assert!(engine
            .handle(
                event(control.clone(), InputPhase::Pressed, 2),
                &layout,
                &agents
            )
            .unwrap()
            .is_empty());
        assert!(engine
            .handle(event(control, InputPhase::Released, 2), &layout, &agents)
            .is_err());
    }

    #[test]
    fn reset_releases_pressed_mic_and_joystick_without_replaying() {
        let mut engine = ControlEngine::default();
        let layout = Layout::official_defaults();
        let agents: Vec<_> = (0..6).map(AgentSlot::empty).collect();
        engine
            .handle(
                event(
                    ControlId::Command {
                        slot: CommandSlot::Act10Act11,
                    },
                    InputPhase::Pressed,
                    1,
                ),
                &layout,
                &agents,
            )
            .unwrap();
        engine
            .handle(
                event(
                    ControlId::Joystick {
                        direction: Direction::Left,
                    },
                    InputPhase::Entered,
                    2,
                ),
                &layout,
                &agents,
            )
            .unwrap();
        let releases = engine.reset(&layout, &agents);
        assert_eq!(releases.len(), 2);
        assert!(releases
            .iter()
            .any(|intent| matches!(intent, Intent::ExecuteBinding { pressed: false, .. })));
        assert!(releases
            .iter()
            .any(|intent| matches!(intent, Intent::JoystickEdge { active: false, .. })));
        assert!(engine.reset(&layout, &agents).is_empty());
    }

    #[test]
    fn disconnect_clears_pressed_edges_without_replay_before_reconnect() {
        let mut engine = ControlEngine::default();
        let layout = Layout::official_defaults();
        let agents: Vec<_> = (0..6).map(AgentSlot::empty).collect();
        let control = ControlId::Command {
            slot: CommandSlot::Act10Act11,
        };
        let first_press = engine
            .handle(
                event(control.clone(), InputPhase::Pressed, 1),
                &layout,
                &agents,
            )
            .unwrap();
        assert_eq!(first_press.len(), 1);

        engine.clear_pressed();

        let reconnected_press = engine
            .handle(event(control, InputPhase::Pressed, 2), &layout, &agents)
            .unwrap();
        assert_eq!(reconnected_press.len(), 1);
        assert!(matches!(
            reconnected_press[0],
            Intent::ExecuteBinding { pressed: true, .. }
        ));
    }
}
