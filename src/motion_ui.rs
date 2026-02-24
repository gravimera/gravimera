use bevy::prelude::*;

use crate::motion::{
    motion_rig_v1_for_prefab, AttackPrimaryMotionAlgorithm, IdleMotionAlgorithm,
    MotionAlgorithmController, MoveMotionAlgorithm,
};
use crate::object::registry::ObjectLibrary;
use crate::prefab_descriptors::PrefabDescriptorLibrary;
use crate::types::{Commandable, ObjectPrefabId, SelectionState};

const PANEL_Z_INDEX: i32 = 940;
const PANEL_WIDTH_PX: f32 = 300.0;
const DOUBLE_CLICK_MAX_SECS: f32 = 0.35;

#[derive(Resource, Debug)]
pub(crate) struct MotionAlgorithmUiState {
    pub(crate) open: bool,
    pub(crate) target: Option<Entity>,
    pub(crate) needs_rebuild: bool,
    last_built_target: Option<Entity>,
    last_click_target: Option<Entity>,
    last_click_time_secs: f32,
}

impl Default for MotionAlgorithmUiState {
    fn default() -> Self {
        Self {
            open: false,
            target: None,
            needs_rebuild: false,
            last_built_target: None,
            last_click_target: None,
            last_click_time_secs: -1.0,
        }
    }
}

impl MotionAlgorithmUiState {
    pub(crate) fn record_click_and_check_double(&mut self, entity: Entity, now_secs: f32) -> bool {
        let is_double = self.last_click_target.is_some_and(|prev| prev == entity)
            && now_secs.is_finite()
            && self.last_click_time_secs.is_finite()
            && (now_secs - self.last_click_time_secs) <= DOUBLE_CLICK_MAX_SECS;

        self.last_click_target = Some(entity);
        self.last_click_time_secs = now_secs;
        is_double
    }

    pub(crate) fn open_for(&mut self, entity: Entity) {
        self.open = true;
        self.target = Some(entity);
        self.needs_rebuild = true;
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.target = None;
        self.needs_rebuild = false;
        self.last_built_target = None;
    }
}

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiRoot;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiTitle;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiSubtitle;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiList;

#[derive(Component)]
pub(crate) struct MotionAlgorithmUiListItem;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MotionAlgorithmUiChannel {
    Idle,
    Move,
    AttackPrimary,
}

#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct MotionAlgorithmUiButton {
    pub(crate) channel: MotionAlgorithmUiChannel,
    pub(crate) algorithm_id: &'static str,
}

pub(crate) fn setup_motion_algorithm_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(44.0),
                right: Val::Px(10.0),
                width: Val::Px(PANEL_WIDTH_PX),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.88)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
            Outline {
                width: Val::Px(1.0),
                color: Color::srgba(0.25, 0.25, 0.30, 0.75),
                offset: Val::Px(0.0),
            },
            ZIndex(PANEL_Z_INDEX),
            Visibility::Hidden,
            MotionAlgorithmUiRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("Motion"),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.95, 0.97)),
                MotionAlgorithmUiTitle,
            ));

            root.spawn((
                Text::new(""),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::srgb(0.80, 0.80, 0.86)),
                MotionAlgorithmUiSubtitle,
            ));

            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                MotionAlgorithmUiList,
            ));

            root.spawn((
                Text::new("Tip: double-click a unit's selection circle to open."),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.65, 0.65, 0.72)),
            ));
        });
}

pub(crate) fn motion_algorithm_ui_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<MotionAlgorithmUiState>,
) {
    if !state.open {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        state.close();
    }
}

pub(crate) fn motion_algorithm_ui_update(
    mut commands: Commands,
    library: Res<ObjectLibrary>,
    descriptors: Res<PrefabDescriptorLibrary>,
    mut state: ResMut<MotionAlgorithmUiState>,
    roots: Query<(Option<&ObjectPrefabId>, Option<&MotionAlgorithmController>)>,
    mut roots_ui: Query<&mut Visibility, With<MotionAlgorithmUiRoot>>,
    mut subtitle: Query<&mut Text, With<MotionAlgorithmUiSubtitle>>,
    list: Query<Entity, With<MotionAlgorithmUiList>>,
    existing_items: Query<Entity, With<MotionAlgorithmUiListItem>>,
) {
    let Ok(mut visibility) = roots_ui.single_mut() else {
        return;
    };

    let Some(target) = state.target else {
        state.open = false;
        *visibility = Visibility::Hidden;
        return;
    };
    if !state.open {
        *visibility = Visibility::Hidden;
        return;
    }

    let Ok((prefab_id, controller)) = roots.get(target) else {
        state.close();
        *visibility = Visibility::Hidden;
        return;
    };
    let Some(prefab_id) = prefab_id else {
        state.close();
        *visibility = Visibility::Hidden;
        return;
    };

    *visibility = Visibility::Visible;

    if state.last_built_target != state.target {
        state.needs_rebuild = true;
    }

    let current_idle = controller
        .map(|c| c.idle_algorithm)
        .unwrap_or(IdleMotionAlgorithm::None);
    let current_move = controller
        .map(|c| c.move_algorithm)
        .unwrap_or(MoveMotionAlgorithm::None);
    let current_attack = controller
        .map(|c| c.attack_primary_algorithm)
        .unwrap_or(AttackPrimaryMotionAlgorithm::None);
    let rig = motion_rig_v1_for_prefab(prefab_id.0, &descriptors)
        .ok()
        .flatten();
    let attack_kind = library
        .get(prefab_id.0)
        .and_then(|def| def.attack.as_ref())
        .map(|a| a.kind);

    if let Ok(mut subtitle) = subtitle.single_mut() {
        let label = descriptors
            .get(prefab_id.0)
            .and_then(|d| d.label.as_deref())
            .or_else(|| library.get(prefab_id.0).map(|d| d.label.as_ref()))
            .unwrap_or("<unknown>");
        let rig_kind = rig.as_ref().map(|r| r.kind_str()).unwrap_or("<none>");
        *subtitle = Text::new(format!(
            "Target: {label}\nRig: {rig_kind}\nIdle: {}\nMove: {}\nAttack: {}",
            current_idle.id_str(),
            current_move.id_str(),
            current_attack.id_str(),
        ));
    }

    if !state.needs_rebuild {
        return;
    }
    state.needs_rebuild = false;
    state.last_built_target = state.target;

    let Ok(list_entity) = list.single() else {
        return;
    };
    for entity in &existing_items {
        commands.entity(entity).try_despawn();
    }

    let idle_algorithms = rig
        .as_ref()
        .map(|r| r.applicable_idle_algorithms())
        .unwrap_or_else(|| vec![IdleMotionAlgorithm::None]);
    let move_algorithms = rig
        .as_ref()
        .map(|r| r.applicable_move_algorithms())
        .unwrap_or_else(|| vec![MoveMotionAlgorithm::None]);
    let attack_algorithms = rig
        .as_ref()
        .map(|r| r.applicable_attack_primary_algorithms(attack_kind))
        .unwrap_or_else(|| vec![AttackPrimaryMotionAlgorithm::None]);

    commands.entity(list_entity).with_children(|list| {
        let section_font = TextFont {
            font_size: 12.0,
            ..default()
        };
        let section_color = TextColor(Color::srgb(0.75, 0.75, 0.82));
        let button_font = TextFont {
            font_size: 14.0,
            ..default()
        };
        let button_color = TextColor(Color::srgb(0.92, 0.92, 0.96));
        let button_bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75));
        let button_border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));

        list.spawn((
            Text::new("Idle"),
            section_font.clone(),
            section_color,
            MotionAlgorithmUiListItem,
        ));
        for algorithm in idle_algorithms {
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                button_bg,
                button_border,
                MotionAlgorithmUiListItem,
                MotionAlgorithmUiButton {
                    channel: MotionAlgorithmUiChannel::Idle,
                    algorithm_id: algorithm.id_str(),
                },
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(algorithm.label()),
                    button_font.clone(),
                    button_color,
                ));
            });
        }

        list.spawn((
            Text::new("Move"),
            section_font.clone(),
            section_color,
            MotionAlgorithmUiListItem,
        ));
        for algorithm in move_algorithms {
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                button_bg,
                button_border,
                MotionAlgorithmUiListItem,
                MotionAlgorithmUiButton {
                    channel: MotionAlgorithmUiChannel::Move,
                    algorithm_id: algorithm.id_str(),
                },
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(algorithm.label()),
                    button_font.clone(),
                    button_color,
                ));
            });
        }

        list.spawn((
            Text::new("Attack"),
            section_font,
            section_color,
            MotionAlgorithmUiListItem,
        ));
        for algorithm in attack_algorithms {
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                button_bg,
                button_border,
                MotionAlgorithmUiListItem,
                MotionAlgorithmUiButton {
                    channel: MotionAlgorithmUiChannel::AttackPrimary,
                    algorithm_id: algorithm.id_str(),
                },
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(algorithm.label()),
                    button_font.clone(),
                    button_color,
                ));
            });
        }
    });
}

pub(crate) fn motion_algorithm_ui_button_styles(
    state: Res<MotionAlgorithmUiState>,
    roots: Query<Option<&MotionAlgorithmController>>,
    mut buttons: Query<
        (
            &Interaction,
            &MotionAlgorithmUiButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<Button>,
    >,
) {
    let Some(target) = state.target else {
        return;
    };
    if !state.open {
        return;
    }

    let selected_idle = roots
        .get(target)
        .ok()
        .flatten()
        .map(|c| c.idle_algorithm)
        .unwrap_or(IdleMotionAlgorithm::None)
        .id_str();
    let selected_move = roots
        .get(target)
        .ok()
        .flatten()
        .map(|c| c.move_algorithm)
        .unwrap_or(MoveMotionAlgorithm::None)
        .id_str();
    let selected_attack = roots
        .get(target)
        .ok()
        .flatten()
        .map(|c| c.attack_primary_algorithm)
        .unwrap_or(AttackPrimaryMotionAlgorithm::None)
        .id_str();

    for (interaction, button, mut bg, mut border) in &mut buttons {
        let selected_id = match button.channel {
            MotionAlgorithmUiChannel::Idle => selected_idle,
            MotionAlgorithmUiChannel::Move => selected_move,
            MotionAlgorithmUiChannel::AttackPrimary => selected_attack,
        };
        let is_selected = button.algorithm_id == selected_id;
        let (base_bg, base_border) = if is_selected {
            (
                Color::srgba(0.10, 0.10, 0.14, 0.88),
                Color::srgba(0.45, 0.45, 0.60, 0.85),
            )
        } else {
            (
                Color::srgba(0.05, 0.05, 0.06, 0.75),
                Color::srgba(0.25, 0.25, 0.30, 0.65),
            )
        };
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(base_bg);
                *border = BorderColor::all(base_border);
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(
                    (base_bg.to_srgba().red + 0.02).min(1.0),
                    (base_bg.to_srgba().green + 0.02).min(1.0),
                    (base_bg.to_srgba().blue + 0.03).min(1.0),
                    base_bg.to_srgba().alpha,
                ));
                *border = BorderColor::all(Color::srgba(
                    (base_border.to_srgba().red + 0.08).min(1.0),
                    (base_border.to_srgba().green + 0.08).min(1.0),
                    (base_border.to_srgba().blue + 0.10).min(1.0),
                    base_border.to_srgba().alpha,
                ));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(
                    (base_bg.to_srgba().red + 0.05).min(1.0),
                    (base_bg.to_srgba().green + 0.05).min(1.0),
                    (base_bg.to_srgba().blue + 0.07).min(1.0),
                    base_bg.to_srgba().alpha,
                ));
                *border = BorderColor::all(Color::srgba(
                    (base_border.to_srgba().red + 0.12).min(1.0),
                    (base_border.to_srgba().green + 0.12).min(1.0),
                    (base_border.to_srgba().blue + 0.14).min(1.0),
                    base_border.to_srgba().alpha,
                ));
            }
        }
    }
}

pub(crate) fn motion_algorithm_ui_button_clicks(
    mut commands: Commands,
    mut state: ResMut<MotionAlgorithmUiState>,
    selection: Res<SelectionState>,
    roots: Query<(&ObjectPrefabId, Option<&MotionAlgorithmController>), With<Commandable>>,
    mut buttons: Query<(&Interaction, &MotionAlgorithmUiButton), Changed<Interaction>>,
) {
    if !state.open {
        return;
    }
    let Some(target) = state.target else {
        return;
    };
    let Ok((target_prefab, _)) = roots.get(target) else {
        return;
    };

    for (interaction, button) in &mut buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let channel = button.channel;
        let alg_label = match channel {
            MotionAlgorithmUiChannel::Idle => button.algorithm_id,
            MotionAlgorithmUiChannel::Move => button.algorithm_id,
            MotionAlgorithmUiChannel::AttackPrimary => button.algorithm_id,
        };

        let mut updated = 0usize;
        let mut targets: Vec<Entity> = selection.selected.iter().copied().collect();
        if !selection.selected.contains(&target) {
            targets.push(target);
        }

        for entity in targets {
            let Ok((prefab_id, controller)) = roots.get(entity) else {
                continue;
            };
            if prefab_id.0 != target_prefab.0 {
                continue;
            }

            let mut next = controller.copied().unwrap_or_default();
            match channel {
                MotionAlgorithmUiChannel::Idle => {
                    let Some(parsed) = IdleMotionAlgorithm::parse(button.algorithm_id) else {
                        continue;
                    };
                    next.idle_algorithm = parsed;
                }
                MotionAlgorithmUiChannel::Move => {
                    let Some(parsed) = MoveMotionAlgorithm::parse(button.algorithm_id) else {
                        continue;
                    };
                    next.move_algorithm = parsed;
                }
                MotionAlgorithmUiChannel::AttackPrimary => {
                    let Some(parsed) = AttackPrimaryMotionAlgorithm::parse(button.algorithm_id)
                    else {
                        continue;
                    };
                    next.attack_primary_algorithm = parsed;
                }
            }

            commands.entity(entity).insert(next);
            updated += 1;
        }

        info!(
            "Motion: set {:?}={} for {} unit(s) of prefab {}",
            channel,
            alg_label,
            updated,
            uuid::Uuid::from_u128(target_prefab.0)
        );

        state.needs_rebuild = true;
    }
}
