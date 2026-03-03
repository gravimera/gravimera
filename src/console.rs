use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use crate::constants::*;
use crate::types::*;

pub(crate) fn console_closed(console: Res<CommandConsole>) -> bool {
    !console.open
}

fn apply_console_command(
    game: &mut Game,
    player_health: &mut Health,
    ratios: &mut SpawnRatios,
    command: &str,
    player_popup_pos: Option<Vec3>,
    health_events: &mut MessageWriter<HealthChangeEvent>,
) {
    let raw = command.trim();
    if raw.is_empty() {
        return;
    }

    let previous_health = player_health.current;

    let normalized = raw.strip_prefix('/').unwrap_or(raw).to_ascii_lowercase();

    let who_prefixes = ["who's your daddy", "whos your daddy"];
    for prefix in who_prefixes {
        if let Some(rest) = normalized.strip_prefix(prefix) {
            let amount_str = rest.trim();
            let amount = if amount_str.is_empty() {
                None
            } else {
                match amount_str.parse::<i32>() {
                    Ok(v) if v > 0 => Some(v),
                    _ => {
                        warn!("Invalid amount for \"who's your daddy\": {amount_str:?}");
                        None
                    }
                }
            };

            match amount {
                None => {
                    *player_health = Health::new(PLAYER_MAX_HEALTH, PLAYER_MAX_HEALTH);
                    game.shotgun_charges = 1000;
                    game.laser_charges = 1000;
                }
                Some(amount) => {
                    player_health.current = player_health.current.saturating_add(amount);
                    player_health.max = player_health.max.saturating_add(amount).max(1);
                    if player_health.current > player_health.max {
                        player_health.max = player_health.current.max(1);
                    }

                    let amount_u32 = amount as u32;
                    game.shotgun_charges = game.shotgun_charges.saturating_add(amount_u32);
                    game.laser_charges = game.laser_charges.saturating_add(amount_u32);
                }
            }

            game.game_over = false;
            if let Some(pos) = player_popup_pos {
                let delta = player_health.current - previous_health;
                if delta != 0 {
                    health_events.write(HealthChangeEvent {
                        world_pos: pos,
                        delta,
                        is_hero: true,
                    });
                }
            }
            return;
        }
    }

    match normalized.as_str() {
        "easy" | "normal" => {
            *game = Game::default();
            *ratios = SpawnRatios::default();
            *player_health = Health::new(PLAYER_MAX_HEALTH, PLAYER_MAX_HEALTH);
            info!("Settings reset to /easy.");
        }
        "hard" | "difficult" => {
            *ratios = SpawnRatios::new(0.60, 0.30, 0.10);
            info!("Spawn ratios set to /hard: Dog 60% | Human 30% | Gundam 10%");
        }
        "hell" => {
            *ratios = SpawnRatios::new(0.10, 0.60, 0.30);
            info!("Spawn ratios set to /hell: Dog 10% | Human 60% | Gundam 30%");
        }
        _ => {}
    }

    if let Some(pos) = player_popup_pos {
        let delta = player_health.current - previous_health;
        if delta != 0 {
            health_events.write(HealthChangeEvent {
                world_pos: pos,
                delta,
                is_hero: true,
            });
        }
    }
}

pub(crate) fn toggle_command_console(
    keys: Res<ButtonInput<KeyCode>>,
    mut console: ResMut<CommandConsole>,
    mut game: ResMut<Game>,
    mut ratios: ResMut<SpawnRatios>,
    mut health_events: MessageWriter<HealthChangeEvent>,
    player_q: Query<&Transform, With<Player>>,
    mut player_health_q: Query<&mut Health, With<Player>>,
) {
    if !(keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter)) {
        return;
    }

    if console.open {
        let popup_pos = player_q
            .single()
            .ok()
            .map(|transform| transform.translation + Vec3::Y * PLAYER_HEALTH_BAR_OFFSET_Y);
        let Ok(mut player_health) = player_health_q.single_mut() else {
            console.open = false;
            console.buffer.clear();
            return;
        };
        apply_console_command(
            &mut game,
            &mut player_health,
            &mut ratios,
            &console.buffer,
            popup_pos,
            &mut health_events,
        );
        console.open = false;
        console.buffer.clear();
    } else {
        console.open = true;
        console.buffer.clear();
    }
}

pub(crate) fn command_console_text_input(
    mut console: ResMut<CommandConsole>,
    mut keyboard: MessageReader<KeyboardInput>,
) {
    for event in keyboard.read() {
        if !console.open {
            continue;
        }
        if event.state != bevy::input::ButtonState::Pressed {
            continue;
        }

        match event.key_code {
            KeyCode::Enter | KeyCode::NumpadEnter => {}
            KeyCode::Backspace => {
                console.buffer.pop();
            }
            KeyCode::Escape => {
                console.open = false;
                console.buffer.clear();
            }
            _ => {
                let Some(text) = &event.text else {
                    continue;
                };
                for ch in text.chars() {
                    if ch.is_control() {
                        continue;
                    }
                    console.buffer.push(ch);
                }
            }
        }
    }
}

pub(crate) fn update_command_console_ui(
    console: Res<CommandConsole>,
    mut roots: Query<&mut Visibility, With<CommandConsoleRoot>>,
    mut texts: Query<&mut Text, With<CommandConsoleText>>,
) {
    let visibility = if console.open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };

    for mut root_visibility in &mut roots {
        *root_visibility = visibility;
    }

    if !console.open {
        return;
    }

    let prompt = format!(
        "Command\n> {}\n\nCommands:\n  /easy\n  /hard\n  /hell",
        console.buffer
    );
    for mut text in &mut texts {
        **text = prompt.clone();
    }
}
