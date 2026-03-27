use bevy::prelude::*;

use crate::object::registry::ObjectLibrary;
use crate::prefab_descriptors::PrefabDescriptorLibrary;
use crate::types::BuildScene;
use uuid::Uuid;

use super::ai::Gen3dAiJob;
use super::state::*;

pub(crate) fn gen3d_side_tab_buttons(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut buttons: Query<
        (&Interaction, &Gen3dSideTabButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    for (interaction, button, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                workshop.side_tab = button.tab();
                *bg = BackgroundColor(Color::srgba(0.08, 0.12, 0.16, 0.92));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.04, 0.04, 0.05, 0.78));
            }
            Interaction::None => {}
        }
    }
}

pub(crate) fn gen3d_update_side_tab_ui(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    mut panels: ParamSet<(
        Query<(&mut Node, &mut Visibility), With<Gen3dStatusPanelRoot>>,
        Query<(&mut Node, &mut Visibility), With<Gen3dPrefabPanelRoot>>,
    )>,
    mut buttons: Query<(&Gen3dSideTabButton, &Interaction, &mut BackgroundColor)>,
    mut texts: Query<(&Gen3dSideTabButtonText, &mut Text)>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    for (mut node, mut vis) in panels.p0().iter_mut() {
        let active = matches!(workshop.side_tab, Gen3dSideTab::Status);
        node.display = if active { Display::Flex } else { Display::None };
        *vis = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (mut node, mut vis) in panels.p1().iter_mut() {
        let active = matches!(workshop.side_tab, Gen3dSideTab::Prefab);
        node.display = if active { Display::Flex } else { Display::None };
        *vis = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }

    for (tab, interaction, mut bg) in buttons.iter_mut() {
        let active = tab.tab() == workshop.side_tab;
        *bg = match (*interaction, active) {
            (Interaction::Pressed, _) => BackgroundColor(Color::srgba(0.08, 0.12, 0.16, 0.92)),
            (_, true) => BackgroundColor(Color::srgba(0.06, 0.10, 0.16, 0.78)),
            (Interaction::Hovered, false) => BackgroundColor(Color::srgba(0.04, 0.04, 0.05, 0.78)),
            _ => BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.70)),
        };
    }

    for (tab, mut text) in &mut texts {
        let label = match tab.tab() {
            Gen3dSideTab::Status => "Status".to_string(),
            Gen3dSideTab::Prefab => "Prefab".to_string(),
        };
        **text = label.into();
    }
}

pub(crate) fn gen3d_update_prefab_details_text(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    job: Res<Gen3dAiJob>,
    descriptors: Res<PrefabDescriptorLibrary>,
    library: Res<ObjectLibrary>,
    mut texts: Query<&mut Text, With<Gen3dPrefabDetailsText>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    if !matches!(workshop.side_tab, Gen3dSideTab::Prefab) {
        return;
    }

    let saved = job.last_saved_prefab_id();
    let overwrite = job.save_overwrite_prefab_id();
    let base = job.edit_base_prefab_id();

    let (prefab_id, source_label) = if let Some(id) = saved {
        (id, "Saved")
    } else if let Some(id) = overwrite {
        (id, "Save target")
    } else if let Some(id) = base {
        (id, "Base")
    } else {
        let mut out = String::new();
        out.push_str("Current Prefab\n\n");
        out.push_str("No prefab selected yet.\n\n");
        out.push_str(
            "Tip: Click Build; successful runs auto-save a prefab descriptor. Save Snapshot appears while generating.\n",
        );

        for mut text in &mut texts {
            **text = out.clone().into();
        }
        return;
    };

    let uuid = Uuid::from_u128(prefab_id).to_string();
    let desc = descriptors.get(prefab_id);

    let name = desc
        .and_then(|d| d.label.as_ref())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(|| uuid.clone());
    let tags = desc.map(|d| d.tags.clone()).unwrap_or_default();
    let roles = desc.map(|d| d.roles.clone()).unwrap_or_default();
    let short = desc
        .and_then(|d| d.text.as_ref())
        .and_then(|t| t.short.as_deref())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());
    let long = desc
        .and_then(|d| d.text.as_ref())
        .and_then(|t| t.long.as_deref())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());
    let gen3d_prompt = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.gen3d.as_ref())
        .and_then(|g| g.prompt.as_deref())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());
    let gen3d_descriptor_meta = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.gen3d.as_ref())
        .and_then(|g| g.extra.get("descriptor_meta_v1"));
    let modified_at_ms = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.modified_at_ms);
    let created_at_ms = desc
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.created_at_ms);

    let mut out = String::new();
    out.push_str("Current Prefab\n\n");

    out.push_str(&format!("Showing: {source_label}\n"));
    out.push_str(&format!("Name: {name}\n"));
    out.push_str(&format!("ID: {uuid}\n"));
    if let Some(modified_at_ms) = modified_at_ms {
        out.push_str(&format!("Modified: {modified_at_ms}\n"));
    }
    if let Some(created_at_ms) = created_at_ms {
        out.push_str(&format!("Created: {created_at_ms}\n"));
    }
    if !roles.is_empty() {
        out.push_str(&format!("Roles: {}\n", roles.join(", ")));
    }
    if !tags.is_empty() {
        out.push_str(&format!("Tags: {}\n", tags.join(", ")));
    }
    if let Some(size) = library.size(prefab_id) {
        out.push_str(&format!(
            "Size (m): [{:.3}, {:.3}, {:.3}]\n",
            size.x, size.y, size.z
        ));
    }

    out.push_str("\nDescriptions\n");
    out.push_str("Short:\n");
    if let Some(short) = short {
        out.push_str(short);
        out.push('\n');
    } else {
        out.push_str("<none>\n");
    }
    out.push('\n');
    out.push_str("Long:\n");
    if let Some(long) = long {
        out.push_str(long);
        out.push('\n');
    } else {
        out.push_str("<none>\n");
    }

    if let Some(gen3d_prompt) = gen3d_prompt {
        out.push('\n');
        out.push_str("Gen3D prompt:\n");
        out.push_str(gen3d_prompt);
        out.push('\n');
    }

    if let Some(meta_json) = gen3d_descriptor_meta {
        let name = meta_json
            .get("name")
            .and_then(|v| v.as_str())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty());
        let short = meta_json
            .get("short")
            .and_then(|v| v.as_str())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty());
        let tags = meta_json
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        out.push('\n');
        out.push_str("AI enriched (descriptor_meta_v1):\n");
        if let Some(name) = name {
            out.push_str(&format!("- name: {name}\n"));
        }
        if let Some(short) = short {
            out.push_str("- short: ");
            out.push_str(short);
            out.push('\n');
        }
        if !tags.is_empty() {
            out.push_str(&format!("- tags: {}\n", tags.join(", ")));
        }
    }

    for mut text in &mut texts {
        **text = out.clone().into();
    }
}

pub(crate) fn gen3d_prefab_scroll_wheel(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<bevy::input::mouse::MouseWheel>,
    mut panels: Query<
        (&ComputedNode, &UiGlobalTransform, &mut ScrollPosition),
        With<Gen3dPrefabScrollPanel>,
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    if !matches!(workshop.side_tab, Gen3dSideTab::Prefab) {
        for _ in mouse_wheel.read() {}
        return;
    }
    let Ok(window) = windows.single() else {
        for _ in mouse_wheel.read() {}
        return;
    };
    let Some(cursor) = window.physical_cursor_position() else {
        for _ in mouse_wheel.read() {}
        return;
    };
    let Ok((node, transform, mut scroll)) = panels.single_mut() else {
        for _ in mouse_wheel.read() {}
        return;
    };

    if !node.contains_point(*transform, cursor) {
        for _ in mouse_wheel.read() {}
        return;
    }

    let mut delta_lines = 0.0f32;
    for ev in mouse_wheel.read() {
        let lines = match ev.unit {
            bevy::input::mouse::MouseScrollUnit::Line => ev.y,
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y / 120.0,
        };
        delta_lines += lines;
    }
    if delta_lines.abs() < 1e-4 {
        return;
    }

    let delta_px = delta_lines * 24.0;
    scroll.y = (scroll.y - delta_px).max(0.0);
}

pub(crate) fn gen3d_update_prefab_scrollbar_ui(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    panels: Query<&ComputedNode, With<Gen3dPrefabScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<Gen3dPrefabScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<Gen3dPrefabScrollbarThumb>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    if !matches!(workshop.side_tab, Gen3dSideTab::Prefab) {
        return;
    }

    let Ok(panel) = panels.single() else {
        return;
    };
    let Ok((track_node, mut track_vis)) = tracks.single_mut() else {
        return;
    };
    let Ok(mut thumb) = thumbs.single_mut() else {
        return;
    };

    let viewport_h = panel.size.y.max(0.0);
    let content_h = panel.content_size.y.max(0.0);
    let track_h = track_node.size.y.max(1.0);

    if viewport_h < 1.0 || content_h < 1.0 {
        *track_vis = Visibility::Hidden;
        return;
    }

    if content_h <= viewport_h + 0.5 {
        *track_vis = Visibility::Hidden;
        thumb.top = Val::Px(0.0);
        thumb.height = Val::Px(track_h);
        return;
    }

    *track_vis = Visibility::Inherited;

    let max_scroll = (content_h - viewport_h).max(1.0);
    let scroll_y = panel.scroll_position.y.clamp(0.0, max_scroll);

    let min_thumb_h = 14.0;
    let thumb_h = (track_h * (viewport_h / content_h)).clamp(min_thumb_h, track_h);
    let thumb_top = (track_h - thumb_h) * (scroll_y / max_scroll);

    thumb.top = Val::Px(thumb_top);
    thumb.height = Val::Px(thumb_h);
}
