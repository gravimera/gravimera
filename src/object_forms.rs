use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::assets::SceneAssets;
use crate::constants::{
    DEFAULT_OBJECT_SIZE_M, SELECTION_RING_RADIUS_MULT, SELECTION_RING_Y_OFFSET,
};
use crate::object::registry::{
    AttachmentDef, MaterialKey, MeshKey, ObjectLibrary, ObjectPartKind, PrimitiveParams,
    PrimitiveVisualDef,
};
use crate::object::visuals;
use crate::rts::draw_circle_xz;
use crate::selection_circle::{self, CursorPickPreference};
use crate::types::*;

mod mapping;

const FORM_TRANSFORM_DURATION_SECS: f32 = 0.55;
const MAX_FLATTEN_DEPTH: usize = 32;

const FORM_BADGE_SIZE_PX: f32 = 26.0;
const FORM_BADGE_FONT_SIZE_PX: f32 = 12.0;
const FORM_BADGE_Z_INDEX: i32 = 350;
const FORM_BADGE_SCREEN_OFFSET_PX: Vec2 = Vec2::new(-18.0, -18.0);

const COPY_CURSOR_BASE_RADIUS_WORLD: f32 = DEFAULT_OBJECT_SIZE_M * 0.5 * SELECTION_RING_RADIUS_MULT;

#[derive(Resource, Default, Debug)]
pub(crate) struct FormCopyState {
    pub(crate) active: bool,
    pub(crate) destinations: Vec<Entity>,
    pub(crate) hovered_source: Option<Entity>,
    pub(crate) started_at_secs: f32,
}

#[derive(Component)]
pub(crate) struct FormTransformAnimation {
    elapsed_secs: f32,
    duration_secs: f32,
    spawned_entities: Vec<Entity>,
    leaf_anims: Vec<FormTransformLeafAnim>,
}

#[derive(Clone, Debug)]
struct FormTransformLeafAnim {
    entity: Entity,
    start: Transform,
    end: Transform,
    material: Option<Handle<StandardMaterial>>,
    start_color: Option<LinearRgba>,
    end_color: Option<LinearRgba>,
}

#[derive(Component)]
pub(crate) struct FormBadgeUi {
    target: Entity,
}

#[derive(Component)]
pub(crate) struct FormBadgeText;

fn ray_plane_intersection_y(ray: Ray3d, y: f32) -> Option<Vec3> {
    let origin = ray.origin;
    let direction = ray.direction;
    let denom = direction.y;
    if denom.abs() < 1e-5 {
        return None;
    }

    let t = (y - origin.y) / denom;
    if t < 0.0 {
        return None;
    }

    Some(origin + direction * t)
}

pub(crate) fn ensure_object_forms_component(
    mut commands: Commands,
    objects: Query<
        (Entity, &ObjectPrefabId),
        (
            Without<Player>,
            Without<ObjectForms>,
            Or<(With<BuildObject>, With<Commandable>)>,
        ),
    >,
) {
    for (entity, prefab_id) in &objects {
        commands
            .entity(entity)
            .insert(ObjectForms::new_single(prefab_id.0));
    }
}

pub(crate) fn object_forms_tab_switch_selected(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    selection: Res<SelectionState>,
    library: Res<ObjectLibrary>,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<visuals::MaterialCache>,
    mut mesh_cache: ResMut<visuals::PrimitiveMeshCache>,
    mut objects: Query<
        (
            Entity,
            &mut Transform,
            &mut ObjectPrefabId,
            &mut ObjectForms,
            Option<&ObjectTint>,
            Option<&Commandable>,
            Option<&BuildObject>,
            Option<&mut Collider>,
            Option<&mut AabbCollider>,
            Option<&mut BuildDimensions>,
            Option<&FormTransformAnimation>,
        ),
        Without<Player>,
    >,
    children_q: Query<&Children>,
) {
    if !keys.just_pressed(KeyCode::Tab) {
        return;
    }

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let animation_duration_secs =
        (FORM_TRANSFORM_DURATION_SECS * if shift { 10.0 } else { 1.0 }).max(1e-3);

    let selected: Vec<Entity> = selection.selected.iter().copied().collect();
    for entity in selected {
        let Ok((
            entity,
            mut transform,
            mut prefab_id,
            mut forms,
            tint,
            commandable,
            build_object,
            collider,
            aabb,
            dimensions,
            active_anim,
        )) = objects.get_mut(entity)
        else {
            continue;
        };
        if active_anim.is_some() {
            continue;
        }

        let is_unit = commandable.is_some() && build_object.is_none();
        let old_prefab_id = prefab_id.0;
        forms.ensure_valid(old_prefab_id);
        if forms.forms.len() <= 1 {
            continue;
        }

        let Some(next_idx) = next_form_index_for_category(&forms, is_unit, &library) else {
            continue;
        };
        if next_idx == forms.active {
            continue;
        }

        forms.active = next_idx;
        let new_prefab_id = forms.active_prefab_id();

        if old_prefab_id == new_prefab_id {
            continue;
        }

        if !is_prefab_category_compatible(is_unit, new_prefab_id, &library) {
            continue;
        }

        apply_switch_and_start_animation(
            &mut commands,
            &mut transform,
            &mut prefab_id,
            collider,
            aabb,
            dimensions,
            tint,
            entity,
            old_prefab_id,
            new_prefab_id,
            animation_duration_secs,
            &library,
            &asset_server,
            &assets,
            &mut meshes,
            &mut materials,
            &mut material_cache,
            &mut mesh_cache,
            &children_q,
        );
    }
}

pub(crate) fn object_forms_copy_mode_start_cancel(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    selection: Res<SelectionState>,
    eligible: Query<(), (Without<Player>, Or<(With<BuildObject>, With<Commandable>)>)>,
    mut copy: ResMut<FormCopyState>,
) {
    // Copy mode is "hold C". If we missed the release (e.g., UI capture / focus changes), exit
    // once C is no longer held.
    if copy.active && !keys.pressed(KeyCode::KeyC) && !keys.just_released(KeyCode::KeyC) {
        copy.active = false;
        copy.destinations.clear();
        copy.hovered_source = None;
        return;
    }

    if keys.just_pressed(KeyCode::Escape) && copy.active {
        copy.active = false;
        copy.destinations.clear();
        copy.hovered_source = None;
        return;
    }

    if copy.active || !keys.just_pressed(KeyCode::KeyC) {
        return;
    }

    let destinations: Vec<Entity> = selection
        .selected
        .iter()
        .copied()
        .filter(|e| eligible.contains(*e))
        .collect();
    if destinations.is_empty() {
        return;
    }

    copy.active = true;
    copy.destinations = destinations;
    copy.hovered_source = None;
    copy.started_at_secs = time.elapsed_secs();
}

fn copy_source_pick_preference(
    destinations: &[Entity],
    categories: &Query<(Option<&Commandable>, Option<&BuildObject>), Without<Player>>,
) -> CursorPickPreference {
    let mut any_unit = false;
    let mut any_build = false;
    for dest in destinations.iter().copied() {
        let Ok((cmd, build)) = categories.get(dest) else {
            continue;
        };
        let is_unit = cmd.is_some() && build.is_none();
        if is_unit {
            any_unit = true;
        } else {
            any_build = true;
        }
        if any_unit && any_build {
            break;
        }
    }

    if any_unit && !any_build {
        CursorPickPreference {
            prefer_units: Some(true),
        }
    } else if any_build && !any_unit {
        CursorPickPreference {
            prefer_units: Some(false),
        }
    } else {
        CursorPickPreference::default()
    }
}

pub(crate) fn object_forms_copy_mode_update_cursor(
    mut gizmos: Gizmos,
    time: Res<Time>,
    mut copy: ResMut<FormCopyState>,
    mut windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &Transform), With<MainCamera>>,
    library: Res<ObjectLibrary>,
    units: Query<
        (
            Entity,
            &Transform,
            Option<&Collider>,
            &ObjectPrefabId,
            Option<&Player>,
        ),
        (With<Commandable>, Without<Player>),
    >,
    builds: Query<
        (Entity, &Transform, &AabbCollider, &ObjectPrefabId),
        (With<BuildObject>, Without<Player>),
    >,
    dest_categories: Query<(Option<&Commandable>, Option<&BuildObject>), Without<Player>>,
) {
    if !copy.active {
        copy.hovered_source = None;
        return;
    }

    let Ok((window, mut cursor_opts)) = windows.single_mut() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        copy.hovered_source = None;
        cursor_opts.visible = true;
        return;
    };

    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };
    let camera_global = GlobalTransform::from(*camera_transform);

    let hovered = selection_circle::pick_under_cursor(
        cursor_pos,
        camera,
        &camera_global,
        &library,
        &units,
        &builds,
        true,
        copy_source_pick_preference(&copy.destinations, &dest_categories),
    );
    copy.hovered_source = hovered.map(|h| h.entity);

    let (wave, alpha) = selection_circle::pulse_wave_alpha(&time, copy.started_at_secs);

    let is_unit = hovered.map(|h| h.is_unit);

    let mut any_compatible = false;
    if let Some(source_is_unit) = is_unit {
        for dest in copy.destinations.iter().copied() {
            let Ok((cmd, build)) = dest_categories.get(dest) else {
                continue;
            };
            let dest_is_unit = cmd.is_some() && build.is_none();
            if dest_is_unit == source_is_unit {
                any_compatible = true;
                break;
            }
        }
    }

    let border_rgb = if is_unit.is_none() {
        (0.95, 0.95, 0.98)
    } else if any_compatible {
        (0.35, 1.00, 0.45)
    } else {
        (1.00, 0.35, 0.35)
    };

    let (world_center, base_radius) = if let Some(hovered) = hovered {
        (hovered.world_center, hovered.world_radius)
    } else {
        let Ok(ray) = camera.viewport_to_world(&camera_global, cursor_pos) else {
            cursor_opts.visible = true;
            return;
        };
        let Some(hit) = ray_plane_intersection_y(ray, SELECTION_RING_Y_OFFSET) else {
            cursor_opts.visible = true;
            return;
        };
        (hit, COPY_CURSOR_BASE_RADIUS_WORLD)
    };

    // Hide the OS cursor while copy mode is active; we render a ground-parallel cursor indicator instead.
    cursor_opts.visible = false;

    let radius = selection_circle::pulse_radius(base_radius, wave);
    draw_circle_xz(
        &mut gizmos,
        world_center,
        radius,
        Color::srgba(border_rgb.0, border_rgb.1, border_rgb.2, alpha),
    );
}

pub(crate) fn object_forms_copy_mode_confirm_on_release(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut copy: ResMut<FormCopyState>,
    library: Res<ObjectLibrary>,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<visuals::MaterialCache>,
    mut mesh_cache: ResMut<visuals::PrimitiveMeshCache>,
    mut objects: Query<
        (
            Entity,
            &mut Transform,
            &mut ObjectPrefabId,
            &mut ObjectForms,
            Option<&ObjectTint>,
            Option<&Commandable>,
            Option<&BuildObject>,
            Option<&mut Collider>,
            Option<&mut AabbCollider>,
            Option<&mut BuildDimensions>,
            Option<&FormTransformAnimation>,
        ),
        Without<Player>,
    >,
    children_q: Query<&Children>,
) {
    if !copy.active || !keys.just_released(KeyCode::KeyC) {
        return;
    }

    let source = copy.hovered_source;
    let destinations = std::mem::take(&mut copy.destinations);
    copy.active = false;
    copy.hovered_source = None;

    let Some(source) = source else {
        return;
    };

    let Ok((
        _,
        _t,
        _source_prefab_id,
        source_forms,
        _tint,
        src_commandable,
        src_build,
        _,
        _,
        _,
        source_anim,
    )) = objects.get_mut(source)
    else {
        return;
    };
    if source_anim.is_some() {
        return;
    }

    let source_is_unit = src_commandable.is_some() && src_build.is_none();
    let source_active_prefab = source_forms.active_prefab_id();

    for dest in destinations {
        if dest == source {
            continue;
        }

        let Ok((
            entity,
            mut transform,
            mut prefab_id,
            mut forms,
            tint,
            dst_commandable,
            dst_build,
            collider,
            aabb,
            dimensions,
            active_anim,
        )) = objects.get_mut(dest)
        else {
            continue;
        };
        if active_anim.is_some() {
            continue;
        }

        let dest_is_unit = dst_commandable.is_some() && dst_build.is_none();
        if dest_is_unit != source_is_unit {
            continue;
        }
        if !is_prefab_category_compatible(dest_is_unit, source_active_prefab, &library) {
            continue;
        }

        let old_prefab_id = prefab_id.0;
        forms.ensure_valid(old_prefab_id);

        let idx = forms.append_dedupe(source_active_prefab);
        forms.active = idx;
        let new_prefab_id = forms.active_prefab_id();

        if old_prefab_id == new_prefab_id {
            continue;
        }

        apply_switch_and_start_animation(
            &mut commands,
            &mut transform,
            &mut prefab_id,
            collider,
            aabb,
            dimensions,
            tint,
            entity,
            old_prefab_id,
            new_prefab_id,
            FORM_TRANSFORM_DURATION_SECS,
            &library,
            &asset_server,
            &assets,
            &mut meshes,
            &mut materials,
            &mut material_cache,
            &mut mesh_cache,
            &children_q,
        );
    }
}

pub(crate) fn tick_form_transform_animations(
    mut commands: Commands,
    time: Res<Time>,
    library: Res<ObjectLibrary>,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<visuals::MaterialCache>,
    mut mesh_cache: ResMut<visuals::PrimitiveMeshCache>,
    mut roots: Query<(
        Entity,
        &ObjectPrefabId,
        Option<&ObjectTint>,
        &mut FormTransformAnimation,
    )>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (root, prefab_id, tint, mut anim) in &mut roots {
        anim.elapsed_secs += dt;
        let t01 = (anim.elapsed_secs / anim.duration_secs.max(1e-3)).clamp(0.0, 1.0);
        let t = smoothstep(t01);

        for leaf in anim.leaf_anims.iter() {
            if let Ok(mut tf) = transforms.get_mut(leaf.entity) {
                *tf = lerp_transform(&leaf.start, &leaf.end, t);
            }

            if let (Some(handle), Some(a), Some(b)) =
                (leaf.material.as_ref(), leaf.start_color, leaf.end_color)
            {
                if let Some(mat) = materials.get_mut(handle) {
                    let c = lerp_linear_rgba(a, b, t);
                    mat.base_color = Color::linear_rgba(c.red, c.green, c.blue, c.alpha);
                    mat.alpha_mode = AlphaMode::Blend;
                }
            }
        }

        if anim.elapsed_secs < anim.duration_secs {
            continue;
        }

        for leaf in anim.leaf_anims.drain(..) {
            if let Some(handle) = leaf.material.as_ref() {
                materials.remove(handle.id());
            }
        }

        for entity in anim.spawned_entities.drain(..) {
            commands.entity(entity).try_despawn();
        }

        // Spawn final visuals for the active prefab.
        let tint = tint.map(|t| t.0);
        let mut ec = commands.entity(root);
        ec.insert(Visibility::Inherited);
        crate::object::visuals::spawn_object_visuals(
            &mut ec,
            &library,
            &asset_server,
            &assets,
            &mut meshes,
            &mut materials,
            &mut material_cache,
            &mut mesh_cache,
            prefab_id.0,
            tint,
        );
        commands.entity(root).remove::<FormTransformAnimation>();
    }
}

pub(crate) fn sync_form_badges(
    mut commands: Commands,
    objects: Query<
        (Entity, &ObjectForms),
        (Without<Player>, Or<(With<BuildObject>, With<Commandable>)>),
    >,
    badges: Query<(Entity, &FormBadgeUi)>,
) {
    let mut by_target: HashMap<Entity, Entity> = HashMap::new();
    for (entity, badge) in &badges {
        by_target.insert(badge.target, entity);
    }

    for (entity, forms) in &objects {
        if forms.forms.len() <= 1 {
            continue;
        }
        if by_target.contains_key(&entity) {
            continue;
        }

        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Px(FORM_BADGE_SIZE_PX),
                    height: Val::Px(FORM_BADGE_SIZE_PX),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(FORM_BADGE_SIZE_PX * 0.5)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.70)),
                BorderColor::all(Color::srgba(0.80, 0.80, 0.90, 0.75)),
                ZIndex(FORM_BADGE_Z_INDEX),
                FormBadgeUi { target: entity },
            ))
            .with_children(|parent| {
                parent.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: FORM_BADGE_FONT_SIZE_PX,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.95, 0.98)),
                    FormBadgeText,
                ));
            });
    }

    for (entity, badge) in &badges {
        let Ok((_target, forms)) = objects.get(badge.target) else {
            commands.entity(entity).try_despawn();
            continue;
        };
        if forms.forms.len() <= 1 {
            commands.entity(entity).try_despawn();
        }
    }
}

pub(crate) fn update_form_badges(
    library: Res<ObjectLibrary>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &Transform), With<MainCamera>>,
    targets: Query<(&Transform, &ObjectPrefabId, &ObjectForms), Without<Player>>,
    mut badges: Query<(&FormBadgeUi, &mut Node, &mut Visibility, &Children)>,
    mut texts: Query<&mut Text, With<FormBadgeText>>,
) {
    let Ok(_window) = windows.single() else {
        return;
    };

    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };
    let camera_global = GlobalTransform::from(*camera_transform);

    for (badge, mut node, mut visibility, children) in &mut badges {
        let Ok((transform, prefab_id, forms)) = targets.get(badge.target) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        if forms.forms.len() <= 1 {
            *visibility = Visibility::Hidden;
            continue;
        }

        let scale_y = transform.scale.y.abs().max(1e-3);
        let height = library
            .size(prefab_id.0)
            .map(|s| s.y * scale_y)
            .unwrap_or(DEFAULT_OBJECT_SIZE_M * scale_y);
        let world_anchor = transform.translation + Vec3::Y * (height.max(0.01) * 0.85);

        let Ok(mut screen) = camera.world_to_viewport(&camera_global, world_anchor) else {
            *visibility = Visibility::Hidden;
            continue;
        };

        screen += FORM_BADGE_SCREEN_OFFSET_PX;
        node.left = Val::Px(screen.x);
        node.top = Val::Px(screen.y);
        *visibility = Visibility::Visible;

        let label = format!("{}/{}", forms.active.saturating_add(1), forms.forms.len());
        for child in children.iter() {
            if let Ok(mut text) = texts.get_mut(child) {
                **text = label.clone().into();
            }
        }
    }
}

fn next_form_index_for_category(
    forms: &ObjectForms,
    is_unit: bool,
    library: &ObjectLibrary,
) -> Option<usize> {
    if forms.forms.len() <= 1 {
        return None;
    }
    let start = forms.active.min(forms.forms.len().saturating_sub(1));
    for offset in 1..=forms.forms.len() {
        let idx = (start + offset) % forms.forms.len();
        let prefab_id = *forms.forms.get(idx)?;
        if is_prefab_category_compatible(is_unit, prefab_id, library) {
            return Some(idx);
        }
    }
    None
}

fn is_prefab_category_compatible(is_unit: bool, prefab_id: u128, library: &ObjectLibrary) -> bool {
    let Some(def) = library.get(prefab_id) else {
        return false;
    };
    def.mobility.is_some() == is_unit
}

fn apply_switch_and_start_animation(
    commands: &mut Commands,
    transform: &mut Transform,
    prefab_id: &mut ObjectPrefabId,
    collider: Option<Mut<'_, Collider>>,
    aabb: Option<Mut<'_, AabbCollider>>,
    dimensions: Option<Mut<'_, BuildDimensions>>,
    tint: Option<&ObjectTint>,
    entity: Entity,
    old_prefab_id: u128,
    new_prefab_id: u128,
    animation_duration_secs: f32,
    library: &ObjectLibrary,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    children_q: &Query<&Children>,
) {
    // Keep bottom Y stable across forms (ground origin can differ by prefab).
    let scale_y = transform.scale.y.abs().max(1e-3);
    let old_origin_y = library.ground_origin_y_or_default(old_prefab_id) * scale_y;
    let new_origin_y = library.ground_origin_y_or_default(new_prefab_id) * scale_y;
    if transform.translation.y.is_finite() && old_origin_y.is_finite() && new_origin_y.is_finite() {
        let bottom_y = transform.translation.y - old_origin_y;
        transform.translation.y = bottom_y + new_origin_y;
    }

    prefab_id.0 = new_prefab_id;

    if let Some(mut collider) = collider {
        collider.radius = compute_unit_radius(library, new_prefab_id, transform);
    }
    if let (Some(mut aabb), Some(mut dimensions)) = (aabb, dimensions) {
        let (half, size) =
            compute_build_object_collider_and_size(library, new_prefab_id, transform);
        aabb.half_extents = half;
        dimensions.size = size;
    }

    commands.entity(entity).insert(Visibility::Inherited);

    begin_form_transform_animation(
        commands,
        entity,
        old_prefab_id,
        new_prefab_id,
        animation_duration_secs,
        tint.map(|t| t.0),
        library,
        asset_server,
        assets,
        meshes,
        materials,
        material_cache,
        mesh_cache,
        children_q,
    );
}

fn compute_unit_radius(library: &ObjectLibrary, prefab_id: u128, transform: &Transform) -> f32 {
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));
    let scale = transform.scale;
    match library.collider(prefab_id) {
        Some(crate::object::registry::ColliderProfile::CircleXZ { radius }) => {
            radius.max(0.01) * scale.x.abs().max(scale.z.abs()).max(0.01)
        }
        Some(crate::object::registry::ColliderProfile::AabbXZ { half_extents }) => {
            let half = Vec2::new(
                half_extents.x.abs().max(0.01) * scale.x.abs().max(0.01),
                half_extents.y.abs().max(0.01) * scale.z.abs().max(0.01),
            );
            half.x.max(half.y)
        }
        _ => {
            let size = Vec2::new(
                (base_size.x * scale.x.abs()).abs().max(0.01),
                (base_size.z * scale.z.abs()).abs().max(0.01),
            );
            (size.x.max(size.y) * 0.5).max(0.01)
        }
    }
}

fn compute_build_object_collider_and_size(
    library: &ObjectLibrary,
    prefab_id: u128,
    transform: &Transform,
) -> (Vec2, Vec3) {
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));

    let (yaw, _pitch, _roll) = transform.rotation.to_euler(EulerRot::YXZ);
    let c = yaw.cos().abs();
    let s = yaw.sin().abs();
    let scale = transform.scale;

    match library.collider(prefab_id) {
        Some(crate::object::registry::ColliderProfile::CircleXZ { radius }) => {
            let r = radius.max(0.01) * scale.x.abs().max(scale.z.abs()).max(0.01);
            (
                Vec2::splat(r),
                Vec3::new(r * 2.0, base_size.y * scale.y.abs(), r * 2.0),
            )
        }
        Some(crate::object::registry::ColliderProfile::AabbXZ { half_extents }) => {
            let half = Vec2::new(
                half_extents.x.abs().max(0.01) * scale.x.abs().max(0.01),
                half_extents.y.abs().max(0.01) * scale.z.abs().max(0.01),
            );
            let rotated = Vec2::new(c * half.x + s * half.y, s * half.x + c * half.y);
            (
                rotated,
                Vec3::new(
                    rotated.x * 2.0,
                    base_size.y * scale.y.abs(),
                    rotated.y * 2.0,
                ),
            )
        }
        _ => {
            let half = Vec2::new(
                (base_size.x * 0.5).abs().max(0.01) * scale.x.abs().max(0.01),
                (base_size.z * 0.5).abs().max(0.01) * scale.z.abs().max(0.01),
            );
            let rotated = Vec2::new(c * half.x + s * half.y, s * half.x + c * half.y);
            (
                rotated,
                Vec3::new(
                    rotated.x * 2.0,
                    base_size.y * scale.y.abs(),
                    rotated.y * 2.0,
                ),
            )
        }
    }
}

fn begin_form_transform_animation(
    commands: &mut Commands,
    root: Entity,
    from_prefab_id: u128,
    to_prefab_id: u128,
    duration_secs: f32,
    tint: Option<Color>,
    library: &ObjectLibrary,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    children_q: &Query<&Children>,
) {
    // Clear any existing visuals under this entity.
    if let Ok(children) = children_q.get(root) {
        for child in children.iter() {
            commands.entity(child).try_despawn();
        }
    }

    let old_leaves = flatten_leaf_visuals(library, from_prefab_id);
    let new_leaves = flatten_leaf_visuals(library, to_prefab_id);

    if old_leaves.is_empty() && new_leaves.is_empty() {
        let mut ec = commands.entity(root);
        ec.insert(Visibility::Inherited);
        crate::object::visuals::spawn_object_visuals(
            &mut ec,
            library,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            to_prefab_id,
            tint,
        );
        return;
    }

    let resolved_old = resolve_leaf_assets(
        &old_leaves,
        tint,
        asset_server,
        assets,
        meshes,
        materials,
        mesh_cache,
    );
    let resolved_new = resolve_leaf_assets(
        &new_leaves,
        tint,
        asset_server,
        assets,
        meshes,
        materials,
        mesh_cache,
    );

    let mapping = build_leaf_mapping(&resolved_old, &resolved_new);
    let mut specs: Vec<LeafAnimSpawnSpec> = Vec::new();

    let mut used_old: HashSet<usize> = HashSet::new();
    let mut used_new: HashSet<usize> = HashSet::new();

    for (old_idx, new_idx, same_key) in mapping.pairs {
        used_old.insert(old_idx);
        used_new.insert(new_idx);
        let old = &resolved_old[old_idx];
        let new = &resolved_new[new_idx];

        if same_key
            && matches!(old.spawn, LeafSpawnKind::Mesh { .. })
            && matches!(new.spawn, LeafSpawnKind::Mesh { .. })
        {
            // Same primitive type: interpolate transform and color.
            specs.push(LeafAnimSpawnSpec::morph_same_type(old, new));
        } else if same_key
            && matches!(old.spawn, LeafSpawnKind::Scene { .. })
            && matches!(new.spawn, LeafSpawnKind::Scene { .. })
        {
            specs.push(LeafAnimSpawnSpec::morph_same_type(old, new));
        } else {
            specs.extend(LeafAnimSpawnSpec::morph_different_type_pair(old, new));
        }
    }

    for (idx, old) in resolved_old.iter().enumerate() {
        if used_old.contains(&idx) {
            continue;
        }
        specs.push(LeafAnimSpawnSpec::fade_out(old));
    }
    for (idx, new) in resolved_new.iter().enumerate() {
        if used_new.contains(&idx) {
            continue;
        }
        specs.push(LeafAnimSpawnSpec::fade_in(new));
    }

    if specs.is_empty() {
        let mut ec = commands.entity(root);
        ec.insert(Visibility::Inherited);
        crate::object::visuals::spawn_object_visuals(
            &mut ec,
            library,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            to_prefab_id,
            tint,
        );
        return;
    }

    let mut spawned_entities = Vec::with_capacity(specs.len());
    let mut leaf_anims = Vec::with_capacity(specs.len());

    commands.entity(root).with_children(|parent| {
        for spec in specs.into_iter() {
            let (entity, anim) = spawn_leaf_anim(parent, spec, materials);
            spawned_entities.push(entity);
            leaf_anims.push(anim);
        }
    });

    commands.entity(root).insert(FormTransformAnimation {
        elapsed_secs: 0.0,
        duration_secs: duration_secs.max(1e-3),
        spawned_entities,
        leaf_anims,
    });
}

#[derive(Clone, Debug)]
enum LeafKindKey {
    Primitive(MeshKey),
    Model(String),
}

impl LeafKindKey {
    fn mesh_rank(key: MeshKey) -> u16 {
        match key {
            MeshKey::UnitCube => 0,
            MeshKey::UnitCylinder => 1,
            MeshKey::UnitCone => 2,
            MeshKey::UnitSphere => 3,
            MeshKey::UnitPlane => 4,
            MeshKey::UnitCapsule => 5,
            MeshKey::UnitConicalFrustum => 6,
            MeshKey::UnitTorus => 7,
            MeshKey::UnitTriangle => 8,
            MeshKey::UnitTetrahedron => 9,
            MeshKey::TreeTrunk => 10,
            MeshKey::TreeCone => 11,
        }
    }
}

impl Ord for LeafKindKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (LeafKindKey::Primitive(a), LeafKindKey::Primitive(b)) => {
                Self::mesh_rank(*a).cmp(&Self::mesh_rank(*b))
            }
            (LeafKindKey::Model(a), LeafKindKey::Model(b)) => a.cmp(b),
            (LeafKindKey::Primitive(_), LeafKindKey::Model(_)) => Ordering::Less,
            (LeafKindKey::Model(_), LeafKindKey::Primitive(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for LeafKindKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for LeafKindKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for LeafKindKey {}

#[derive(Clone, Debug)]
struct LeafProto {
    key: LeafKindKey,
    transform: Transform,
    kind: LeafKind,
}

#[derive(Clone, Debug)]
enum LeafKind {
    Primitive(PrimitiveVisualDef),
    Model(String),
}

fn flatten_leaf_visuals(library: &ObjectLibrary, root_object_id: u128) -> Vec<LeafProto> {
    let mut out = Vec::new();
    let mut stack = Vec::new();
    flatten_leaf_visuals_inner(
        library,
        root_object_id,
        Transform::IDENTITY,
        0,
        &mut stack,
        &mut out,
    );
    out
}

fn flatten_leaf_visuals_inner(
    library: &ObjectLibrary,
    object_id: u128,
    parent_transform: Transform,
    depth: usize,
    stack: &mut Vec<u128>,
    out: &mut Vec<LeafProto>,
) {
    if depth > MAX_FLATTEN_DEPTH {
        warn!("Object forms: max flatten depth exceeded at object_id {object_id:#x}");
        return;
    }
    if stack.contains(&object_id) {
        warn!(
            "Object forms: detected composition cycle while flattening: {:?} -> {object_id:#x}",
            stack
        );
        return;
    }

    let Some(def) = library.get(object_id) else {
        return;
    };

    stack.push(object_id);
    for part in def.parts.iter() {
        let mut child_local = part.transform;
        if let Some(attachment) = part.attachment.as_ref() {
            child_local = resolve_attachment_transform(library, def, part, attachment)
                .unwrap_or_else(|| part.transform);
        }
        let child_accum = mul_transform(&parent_transform, &child_local);

        match &part.kind {
            ObjectPartKind::ObjectRef { object_id } => {
                flatten_leaf_visuals_inner(library, *object_id, child_accum, depth + 1, stack, out);
            }
            ObjectPartKind::Primitive { primitive } => {
                let mesh_key = match primitive {
                    PrimitiveVisualDef::Mesh { mesh, .. } => *mesh,
                    PrimitiveVisualDef::Primitive { mesh, .. } => *mesh,
                };
                out.push(LeafProto {
                    key: LeafKindKey::Primitive(mesh_key),
                    transform: child_accum,
                    kind: LeafKind::Primitive(primitive.clone()),
                });
            }
            ObjectPartKind::Model { scene } => out.push(LeafProto {
                key: LeafKindKey::Model(scene.to_string()),
                transform: child_accum,
                kind: LeafKind::Model(scene.to_string()),
            }),
        }
    }
    stack.pop();
}

fn resolve_attachment_transform(
    library: &ObjectLibrary,
    parent_def: &crate::object::registry::ObjectDef,
    part: &crate::object::registry::ObjectPartDef,
    attachment: &AttachmentDef,
) -> Option<Transform> {
    let parent_anchor = anchor_transform(parent_def, attachment.parent_anchor.as_ref())?;
    let child_anchor = match &part.kind {
        ObjectPartKind::ObjectRef { object_id } => library
            .get(*object_id)
            .and_then(|def| anchor_transform(def, attachment.child_anchor.as_ref()))
            .unwrap_or(Transform::IDENTITY),
        _ => Transform::IDENTITY,
    };

    let parent_mat = parent_anchor.to_matrix();
    let offset_mat = part.transform.to_matrix();
    let child_mat = child_anchor.to_matrix();
    let child_inv = child_mat.inverse();

    let composed = parent_mat * offset_mat * child_inv;
    crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
}

fn anchor_transform(def: &crate::object::registry::ObjectDef, name: &str) -> Option<Transform> {
    if name == "origin" {
        return Some(Transform::IDENTITY);
    }
    def.anchors
        .iter()
        .find(|a| a.name.as_ref() == name)
        .map(|a| a.transform)
}

fn mul_transform(a: &Transform, b: &Transform) -> Transform {
    let composed = a.to_matrix() * b.to_matrix();
    crate::geometry::mat4_to_transform_allow_degenerate_scale(composed).unwrap_or(*b)
}

fn lerp_transform(a: &Transform, b: &Transform, alpha: f32) -> Transform {
    let translation = a.translation.lerp(b.translation, alpha);
    let rotation = a.rotation.slerp(b.rotation, alpha).normalize();
    let scale = a.scale.lerp(b.scale, alpha);
    Transform {
        translation,
        rotation,
        scale,
    }
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp_linear_rgba(a: LinearRgba, b: LinearRgba, t: f32) -> LinearRgba {
    LinearRgba {
        red: a.red + (b.red - a.red) * t,
        green: a.green + (b.green - a.green) * t,
        blue: a.blue + (b.blue - a.blue) * t,
        alpha: a.alpha + (b.alpha - a.alpha) * t,
    }
}

#[derive(Clone, Debug)]
enum LeafSpawnKind {
    Mesh {
        mesh: Handle<Mesh>,
        material_proto: StandardMaterial,
        base_color: LinearRgba,
    },
    Scene {
        scene: Handle<Scene>,
    },
}

#[derive(Clone, Debug)]
struct LeafResolved {
    key: LeafKindKey,
    transform: Transform,
    spawn: LeafSpawnKind,
}

fn resolve_leaf_assets(
    leaves: &[LeafProto],
    tint: Option<Color>,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &Assets<StandardMaterial>,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
) -> Vec<LeafResolved> {
    let mut out = Vec::with_capacity(leaves.len());
    let tint = tint.unwrap_or(Color::WHITE);
    for leaf in leaves {
        match &leaf.kind {
            LeafKind::Primitive(primitive) => {
                let Some((mesh, material_proto, base_color)) =
                    resolve_primitive_proto(primitive, tint, assets, meshes, materials, mesh_cache)
                else {
                    continue;
                };
                out.push(LeafResolved {
                    key: leaf.key.clone(),
                    transform: leaf.transform,
                    spawn: LeafSpawnKind::Mesh {
                        mesh,
                        material_proto,
                        base_color,
                    },
                });
            }
            LeafKind::Model(scene) => {
                let handle: Handle<Scene> = asset_server.load(scene.clone());
                out.push(LeafResolved {
                    key: leaf.key.clone(),
                    transform: leaf.transform,
                    spawn: LeafSpawnKind::Scene { scene: handle },
                });
            }
        }
    }
    out
}

fn resolve_primitive_proto(
    visual: &PrimitiveVisualDef,
    tint: Color,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &Assets<StandardMaterial>,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
) -> Option<(Handle<Mesh>, StandardMaterial, LinearRgba)> {
    match visual {
        PrimitiveVisualDef::Mesh { mesh, material } => {
            let mesh_handle = resolve_mesh(*mesh, assets)?;
            let base_handle = resolve_material(*material, assets)?;
            let mut proto = materials.get(&base_handle).cloned().unwrap_or_default();
            proto.base_color = multiply_color(proto.base_color, tint);
            proto.alpha_mode = AlphaMode::Blend;
            let linear = proto.base_color.to_linear();
            Some((mesh_handle, proto, linear))
        }
        PrimitiveVisualDef::Primitive {
            mesh,
            params,
            color,
            unlit,
        } => {
            let mesh_handle = match params {
                Some(params)
                    if matches!(
                        (*mesh, params),
                        (MeshKey::UnitCapsule, PrimitiveParams::Capsule { .. })
                            | (
                                MeshKey::UnitConicalFrustum,
                                PrimitiveParams::ConicalFrustum { .. }
                            )
                            | (MeshKey::UnitTorus, PrimitiveParams::Torus { .. })
                    ) =>
                {
                    mesh_cache.get_or_create(meshes, *params)
                }
                Some(_) => resolve_mesh(*mesh, assets)?,
                None => resolve_mesh(*mesh, assets)?,
            };

            let c = multiply_color(*color, tint);
            let mut proto = StandardMaterial {
                base_color: c,
                unlit: *unlit,
                alpha_mode: AlphaMode::Blend,
                metallic: 0.0,
                perceptual_roughness: 0.92,
                ..default()
            };
            if c.to_srgba().alpha < 1.0 {
                proto.alpha_mode = AlphaMode::Blend;
            }
            let linear = c.to_linear();
            Some((mesh_handle, proto, linear))
        }
    }
}

fn resolve_mesh(key: MeshKey, assets: &SceneAssets) -> Option<Handle<Mesh>> {
    Some(match key {
        MeshKey::UnitCube => assets.unit_cube_mesh.clone(),
        MeshKey::UnitCylinder => assets.unit_cylinder_mesh.clone(),
        MeshKey::UnitCone => assets.unit_cone_mesh.clone(),
        MeshKey::UnitSphere => assets.unit_sphere_mesh.clone(),
        MeshKey::UnitPlane => assets.unit_plane_mesh.clone(),
        MeshKey::UnitCapsule => assets.unit_capsule_mesh.clone(),
        MeshKey::UnitConicalFrustum => assets.unit_conical_frustum_mesh.clone(),
        MeshKey::UnitTorus => assets.unit_torus_mesh.clone(),
        MeshKey::UnitTriangle => assets.unit_triangle_mesh.clone(),
        MeshKey::UnitTetrahedron => assets.unit_tetrahedron_mesh.clone(),
        MeshKey::TreeTrunk => assets.tree_trunk_mesh.clone(),
        MeshKey::TreeCone => assets.tree_cone_mesh.clone(),
    })
}

fn resolve_material(key: MaterialKey, assets: &SceneAssets) -> Option<Handle<StandardMaterial>> {
    let material = match key {
        MaterialKey::BuildBlock { index } => assets
            .build_block_materials
            .get(index)
            .cloned()
            .or_else(|| assets.build_block_materials.first().cloned())?,
        MaterialKey::FenceStake => assets.fence_stake_material.clone(),
        MaterialKey::FenceStick => assets.fence_stick_material.clone(),
        MaterialKey::TreeTrunk { variant } => assets
            .tree_trunk_materials
            .get(variant)
            .cloned()
            .or_else(|| assets.tree_trunk_materials.first().cloned())?,
        MaterialKey::TreeMain { variant } => assets
            .tree_main_materials
            .get(variant)
            .cloned()
            .or_else(|| assets.tree_main_materials.first().cloned())?,
        MaterialKey::TreeCrown { variant } => assets
            .tree_crown_materials
            .get(variant)
            .cloned()
            .or_else(|| assets.tree_crown_materials.first().cloned())?,
    };
    Some(material)
}

fn multiply_color(base: Color, tint: Color) -> Color {
    let a = base.to_srgba();
    let b = tint.to_srgba();
    Color::srgba(
        a.red * b.red,
        a.green * b.green,
        a.blue * b.blue,
        a.alpha * b.alpha,
    )
}

#[derive(Debug)]
struct LeafMapping {
    pairs: Vec<(usize, usize, bool)>,
}

fn build_leaf_mapping(old: &[LeafResolved], new: &[LeafResolved]) -> LeafMapping {
    // Prefer v2 (geometry-aware grouped assignment); keep v1 as a fallback.
    let mapping = mapping::build_leaf_mapping_v2_grouped(old, new);
    if mapping.pairs.is_empty() && !old.is_empty() && !new.is_empty() {
        return build_leaf_mapping_v1(old, new);
    }
    mapping
}

fn build_leaf_mapping_v1(old: &[LeafResolved], new: &[LeafResolved]) -> LeafMapping {
    let mut old_by_key: BTreeMap<LeafKindKey, Vec<usize>> = BTreeMap::new();
    let mut new_by_key: BTreeMap<LeafKindKey, Vec<usize>> = BTreeMap::new();
    for (idx, leaf) in old.iter().enumerate() {
        old_by_key.entry(leaf.key.clone()).or_default().push(idx);
    }
    for (idx, leaf) in new.iter().enumerate() {
        new_by_key.entry(leaf.key.clone()).or_default().push(idx);
    }

    let mut used_old = vec![false; old.len()];
    let mut used_new = vec![false; new.len()];
    let mut pairs = Vec::new();

    for (key, old_idxs) in old_by_key.iter() {
        let Some(new_idxs) = new_by_key.get(key) else {
            continue;
        };
        let count = old_idxs.len().min(new_idxs.len());
        for i in 0..count {
            let oi = old_idxs[i];
            let ni = new_idxs[i];
            used_old[oi] = true;
            used_new[ni] = true;
            pairs.push((oi, ni, true));
        }
    }

    let remaining_old: Vec<usize> = used_old
        .iter()
        .enumerate()
        .filter_map(|(idx, used)| (!*used).then_some(idx))
        .collect();
    let remaining_new: Vec<usize> = used_new
        .iter()
        .enumerate()
        .filter_map(|(idx, used)| (!*used).then_some(idx))
        .collect();
    let count = remaining_old.len().min(remaining_new.len());
    for i in 0..count {
        pairs.push((remaining_old[i], remaining_new[i], false));
    }

    LeafMapping { pairs }
}

#[derive(Clone, Debug)]
struct LeafAnimSpawnSpec {
    spawn: LeafSpawnKind,
    start: Transform,
    end: Transform,
    start_color: Option<LinearRgba>,
    end_color: Option<LinearRgba>,
}

impl LeafAnimSpawnSpec {
    fn morph_same_type(old: &LeafResolved, new: &LeafResolved) -> Self {
        let (spawn, start_color, end_color) = match &old.spawn {
            LeafSpawnKind::Mesh {
                mesh,
                material_proto,
                base_color,
            } => (
                LeafSpawnKind::Mesh {
                    mesh: mesh.clone(),
                    material_proto: material_proto.clone(),
                    base_color: *base_color,
                },
                Some(*base_color),
                match &new.spawn {
                    LeafSpawnKind::Mesh { base_color, .. } => Some(*base_color),
                    _ => Some(*base_color),
                },
            ),
            LeafSpawnKind::Scene { scene } => (
                LeafSpawnKind::Scene {
                    scene: scene.clone(),
                },
                None,
                None,
            ),
        };

        Self {
            spawn,
            start: old.transform,
            end: new.transform,
            start_color,
            end_color,
        }
    }

    fn morph_different_type_pair(old: &LeafResolved, new: &LeafResolved) -> Vec<Self> {
        let mut out = Vec::with_capacity(2);

        // Old: shrink/fade out towards the new transform.
        let mut old_end = new.transform;
        old_end.scale = Vec3::ZERO;
        let (old_spawn, old_start_color) = spawn_and_color(&old.spawn, 1.0);
        out.push(Self {
            spawn: old_spawn,
            start: old.transform,
            end: old_end,
            start_color: old_start_color,
            end_color: old_start_color.map(|c| LinearRgba { alpha: 0.0, ..c }),
        });

        // New: grow/fade in from the old transform.
        let mut new_start = old.transform;
        new_start.scale = Vec3::ZERO;
        let (new_spawn, new_end_color) = spawn_and_color(&new.spawn, 1.0);
        out.push(Self {
            spawn: new_spawn,
            start: new_start,
            end: new.transform,
            start_color: new_end_color.map(|c| LinearRgba { alpha: 0.0, ..c }),
            end_color: new_end_color,
        });

        out
    }

    fn fade_out(old: &LeafResolved) -> Self {
        let mut end = old.transform;
        end.translation = Vec3::ZERO;
        end.scale = Vec3::ZERO;
        let (spawn, color) = spawn_and_color(&old.spawn, 1.0);
        Self {
            spawn,
            start: old.transform,
            end,
            start_color: color,
            end_color: color.map(|c| LinearRgba { alpha: 0.0, ..c }),
        }
    }

    fn fade_in(new: &LeafResolved) -> Self {
        let mut start = new.transform;
        start.translation = Vec3::ZERO;
        start.scale = Vec3::ZERO;
        let (spawn, color) = spawn_and_color(&new.spawn, 1.0);
        Self {
            spawn,
            start,
            end: new.transform,
            start_color: color.map(|c| LinearRgba { alpha: 0.0, ..c }),
            end_color: color,
        }
    }
}

fn spawn_and_color(spawn: &LeafSpawnKind, _alpha: f32) -> (LeafSpawnKind, Option<LinearRgba>) {
    match spawn {
        LeafSpawnKind::Mesh {
            mesh,
            material_proto,
            base_color,
        } => (
            LeafSpawnKind::Mesh {
                mesh: mesh.clone(),
                material_proto: material_proto.clone(),
                base_color: *base_color,
            },
            Some(*base_color),
        ),
        LeafSpawnKind::Scene { scene } => (
            LeafSpawnKind::Scene {
                scene: scene.clone(),
            },
            None,
        ),
    }
}

fn spawn_leaf_anim(
    parent: &mut ChildSpawnerCommands,
    spec: LeafAnimSpawnSpec,
    materials: &mut Assets<StandardMaterial>,
) -> (Entity, FormTransformLeafAnim) {
    let mut ec = parent.spawn((spec.start, Visibility::Inherited));
    let mut material_handle = None;

    match &spec.spawn {
        LeafSpawnKind::Mesh {
            mesh,
            material_proto,
            ..
        } => {
            let mut proto = material_proto.clone();
            if let Some(c) = spec.start_color {
                proto.base_color = Color::linear_rgba(c.red, c.green, c.blue, c.alpha);
            }
            proto.alpha_mode = AlphaMode::Blend;
            let handle = materials.add(proto);
            ec.insert((Mesh3d(mesh.clone()), MeshMaterial3d(handle.clone())));
            material_handle = Some(handle);
        }
        LeafSpawnKind::Scene { scene } => {
            ec.insert(SceneRoot(scene.clone()));
        }
    }

    let entity = ec.id();
    (
        entity,
        FormTransformLeafAnim {
            entity,
            start: spec.start,
            end: spec.end,
            material: material_handle,
            start_color: spec.start_color,
            end_color: spec.end_color,
        },
    )
}
