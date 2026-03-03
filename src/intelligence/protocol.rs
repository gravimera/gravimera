use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub ok: bool,
    pub error: String,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: error.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub name: String,
    pub version: String,
    pub protocol_version: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BudgetCaps {
    pub max_nearby_entities: u32,
    pub max_events_per_delivery: u32,
    pub max_commands_per_tick: u32,
    pub max_speech_bytes_per_tick: u32,
}

impl Default for BudgetCaps {
    fn default() -> Self {
        Self {
            max_nearby_entities: 32,
            max_events_per_delivery: 64,
            max_commands_per_tick: 8,
            max_speech_bytes_per_tick: 256,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TickInputMeta {
    pub nearby_entities_dropped: u32,
    pub events_dropped: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TickOutputMeta {
    pub commands_dropped: u32,
    pub speech_bytes_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainModuleDescriptor {
    /// Stable, human-readable id (e.g. "demo.orbit.v1").
    pub module_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadModuleRequest {
    pub protocol_version: u32,
    pub module_descriptor: BrainModuleDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadModuleResponse {
    pub ok: bool,
    pub protocol_version: u32,
    pub module_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnBrainInstanceRequest {
    pub protocol_version: u32,
    pub realm_id: String,
    pub scene_id: String,
    pub unit_instance_id: String,
    pub module_id: String,
    pub config: serde_json::Value,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnBrainInstanceResponse {
    pub ok: bool,
    pub protocol_version: u32,
    pub brain_instance_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DespawnBrainInstanceRequest {
    pub protocol_version: u32,
    pub brain_instance_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DespawnBrainInstanceResponse {
    pub ok: bool,
    pub protocol_version: u32,
    pub despawned: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickManyRequest {
    pub protocol_version: u32,
    pub items: Vec<TickManyItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickManyItem {
    pub brain_instance_id: String,
    pub tick_input: TickInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickManyResponse {
    pub ok: bool,
    pub protocol_version: u32,
    pub outputs: Vec<TickManyOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickManyOutput {
    pub brain_instance_id: String,
    pub tick_output: Option<TickOutput>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfState {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub vel: [f32; 3],
    pub health: Option<i32>,
    pub stamina: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearbyEntity {
    pub entity_instance_id: String,
    pub kind: String,
    pub rel_pos: [f32; 3],
    pub rel_vel: [f32; 3],
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrainEvent {
    EventsDropped { count: u32 },
    SeenEnter { entity_instance_id: String },
    SeenExit { entity_instance_id: String },
    CommandResult {
        command_id: String,
        ok: bool,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickInput {
    pub realm_id: String,
    pub scene_id: String,
    pub unit_instance_id: String,
    pub dt_ms: u32,
    pub tick_index: u64,
    pub rng_seed: u64,
    pub self_state: SelfState,
    pub nearby_entities: Vec<NearbyEntity>,
    pub events: Vec<BrainEvent>,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub meta: TickInputMeta,
}

impl TickInput {
    pub fn clamp_in_place(&mut self, caps: BudgetCaps) {
        let max_nearby = caps.max_nearby_entities as usize;
        if self.nearby_entities.len() > max_nearby {
            let dropped = (self.nearby_entities.len() - max_nearby) as u32;
            self.nearby_entities.truncate(max_nearby);
            self.meta.nearby_entities_dropped =
                self.meta.nearby_entities_dropped.saturating_add(dropped);
        }

        let max_events = caps.max_events_per_delivery as usize;
        if max_events == 0 {
            if !self.events.is_empty() {
                self.meta.events_dropped = self.meta.events_dropped.saturating_add(
                    self.events
                        .len()
                        .try_into()
                        .unwrap_or(u32::MAX),
                );
                self.events.clear();
            }
            return;
        }

        if self.events.len() > max_events {
            let dropped_total = (self.events.len() - (max_events.saturating_sub(1))) as u32;
            self.events.truncate(max_events.saturating_sub(1));
            self.events.push(BrainEvent::EventsDropped {
                count: dropped_total,
            });
            self.meta.events_dropped = self.meta.events_dropped.saturating_add(dropped_total);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrainCommand {
    MoveTo {
        pos: [f32; 3],
        valid_until_tick: Option<u64>,
    },
    SetMove {
        vec2: [f32; 2],
        valid_until_tick: Option<u64>,
    },
    Say {
        channel: String,
        text: String,
        target_id: Option<String>,
        valid_until_tick: Option<u64>,
    },
    SleepForTicks { ticks: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickOutput {
    pub commands: Vec<BrainCommand>,
    #[serde(default)]
    pub meta: TickOutputMeta,
}

impl TickOutput {
    pub fn clamp_in_place(&mut self, caps: BudgetCaps) {
        let max_commands = caps.max_commands_per_tick as usize;
        if self.commands.len() > max_commands {
            let dropped = (self.commands.len() - max_commands) as u32;
            self.commands.truncate(max_commands);
            self.meta.commands_dropped = self.meta.commands_dropped.saturating_add(dropped);
        }

        let max_speech_bytes = caps.max_speech_bytes_per_tick as usize;
        if max_speech_bytes == 0 {
            for cmd in &mut self.commands {
                if let BrainCommand::Say { text, .. } = cmd {
                    if !text.is_empty() {
                        text.clear();
                        self.meta.speech_bytes_truncated = true;
                    }
                }
            }
            return;
        }

        let mut used = 0usize;
        for cmd in &mut self.commands {
            let BrainCommand::Say { text, .. } = cmd else {
                continue;
            };
            let bytes = text.as_bytes().len();
            if used >= max_speech_bytes {
                if !text.is_empty() {
                    text.clear();
                    self.meta.speech_bytes_truncated = true;
                }
                continue;
            }
            if used + bytes <= max_speech_bytes {
                used += bytes;
                continue;
            }

            // Truncate on a UTF-8 boundary.
            let remaining = max_speech_bytes - used;
            let mut cut = remaining.min(text.len());
            while cut > 0 && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            if cut < text.len() {
                text.truncate(cut);
                self.meta.speech_bytes_truncated = true;
            }
            used = max_speech_bytes;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_input_clamps_events_and_nearby() {
        let mut input = TickInput {
            realm_id: "realm".into(),
            scene_id: "scene".into(),
            unit_instance_id: "unit".into(),
            dt_ms: 16,
            tick_index: 123,
            rng_seed: 999,
            self_state: SelfState {
                pos: [0.0, 0.0, 0.0],
                yaw: 0.0,
                vel: [0.0, 0.0, 0.0],
                health: None,
                stamina: None,
            },
            nearby_entities: (0..10)
                .map(|i| NearbyEntity {
                    entity_instance_id: format!("e{i}"),
                    kind: "thing".into(),
                    rel_pos: [0.0, 0.0, 0.0],
                    rel_vel: [0.0, 0.0, 0.0],
                    tags: vec![],
                })
                .collect(),
            events: (0..10)
                .map(|_| BrainEvent::SeenEnter {
                    entity_instance_id: "x".into(),
                })
                .collect(),
            capabilities: vec!["brain.move".into()],
            meta: TickInputMeta::default(),
        };

        input.clamp_in_place(BudgetCaps {
            max_nearby_entities: 3,
            max_events_per_delivery: 4,
            ..BudgetCaps::default()
        });

        assert_eq!(input.nearby_entities.len(), 3);
        assert_eq!(input.meta.nearby_entities_dropped, 7);

        // Keep 3 original events + 1 synthetic EventsDropped.
        assert_eq!(input.events.len(), 4);
        assert_eq!(input.meta.events_dropped, 7);
        assert!(matches!(
            input.events.last().unwrap(),
            BrainEvent::EventsDropped { count: 7 }
        ));
    }

    #[test]
    fn tick_output_clamps_commands_and_speech_bytes() {
        let mut output = TickOutput {
            commands: vec![
                BrainCommand::MoveTo {
                    pos: [1.0, 0.0, 2.0],
                    valid_until_tick: None,
                },
                BrainCommand::Say {
                    channel: "ambient".into(),
                    text: "hello".into(),
                    target_id: None,
                    valid_until_tick: None,
                },
                BrainCommand::Say {
                    channel: "ambient".into(),
                    text: "world".into(),
                    target_id: None,
                    valid_until_tick: None,
                },
                BrainCommand::MoveTo {
                    pos: [3.0, 0.0, 4.0],
                    valid_until_tick: None,
                },
            ],
            meta: TickOutputMeta::default(),
        };

        output.clamp_in_place(BudgetCaps {
            max_commands_per_tick: 3,
            max_speech_bytes_per_tick: 7,
            ..BudgetCaps::default()
        });

        assert_eq!(output.commands.len(), 3);
        assert_eq!(output.meta.commands_dropped, 1);

        // "hello" (5) fits, "world" (5) is truncated to 2 bytes ("wo").
        let texts: Vec<String> = output
            .commands
            .iter()
            .filter_map(|c| match c {
                BrainCommand::Say { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["hello".to_string(), "wo".to_string()]);
        assert!(output.meta.speech_bytes_truncated);
    }
}
