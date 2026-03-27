use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::window::FileDragAndDrop;
use std::path::PathBuf;

use crate::types::BuildScene;

use super::ai::Gen3dAiJob;
use super::state::*;

pub(crate) fn gen3d_update_images_inline_visibility(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    mut panels: Query<(&mut Node, &mut Visibility), With<Gen3dImagesInlinePanel>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    let has_images = !workshop.images.is_empty();
    let has_prompt = !workshop.prompt.trim().is_empty();
    let show = has_images || has_prompt;
    for (mut node, mut vis) in &mut panels {
        if show {
            node.display = Display::Flex;
            *vis = Visibility::Visible;
            node.width = Val::Px(if has_images { 240.0 } else { 72.0 });
        } else {
            node.display = Display::None;
            *vis = Visibility::Hidden;
        }
    }
}

pub(crate) fn gen3d_clear_images_button(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut job: ResMut<Gen3dAiJob>,
    mut scroll_panels: Query<&mut ScrollPosition, With<Gen3dPromptScrollPanel>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dClearImagesButton>),
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                if job.is_running() {
                    workshop.error = Some("Cannot clear while building.".into());
                    continue;
                }
                workshop.images.clear();
                workshop.prompt.clear();
                workshop.image_viewer = None;
                workshop.error = None;
                workshop.prompt_focused = true;
                workshop.prompt_scrollbar_drag = None;
                if let Ok(mut scroll) = scroll_panels.single_mut() {
                    scroll.y = 0.0;
                }
                job.reset_session();
                if !job.is_running() {
                    workshop.status = format!(
                        "Drop 0–{} images (optional) and/or type a prompt, then click Build.",
                        super::GEN3D_MAX_IMAGES
                    );
                }
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.78));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.70));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65));
            }
        }
    }
}

pub(crate) fn gen3d_handle_drag_and_drop(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    job: Res<Gen3dAiJob>,
    mut images: ResMut<Assets<Image>>,
    mut drag_and_drop: bevy::ecs::message::MessageReader<FileDragAndDrop>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    for event in drag_and_drop.read() {
        let FileDragAndDrop::DroppedFile { path_buf, .. } = event else {
            continue;
        };

        if job.is_running() {
            workshop.error =
                Some("Please wait for generation to finish before adding images.".into());
            continue;
        }

        if workshop.images.len() >= super::GEN3D_MAX_IMAGES {
            workshop.error = Some(format!(
                "Too many images. MVP supports up to {}.",
                super::GEN3D_MAX_IMAGES
            ));
            continue;
        }

        if !is_supported_image_path(path_buf) {
            workshop.error = Some(format!(
                "Unsupported file type: {} (use png/jpg/webp)",
                path_buf.display()
            ));
            continue;
        }

        match std::fs::metadata(path_buf) {
            Ok(meta) => {
                let bytes = meta.len();
                if bytes >= super::GEN3D_MAX_IMAGE_BYTES {
                    let mib = bytes as f64 / (1024.0 * 1024.0);
                    workshop.error = Some(format!(
                        "Image is too large: {:.2} MiB. Max per image is <5 MiB.",
                        mib
                    ));
                    continue;
                }
            }
            Err(err) => {
                workshop.error = Some(format!(
                    "Failed to read image file metadata for {}: {err}",
                    path_buf.display()
                ));
                continue;
            }
        }

        match load_gen3d_ui_image(&mut images, path_buf) {
            Ok((handle, aspect)) => {
                workshop.images.push(Gen3dImageRef {
                    path: path_buf.clone(),
                    ui_image: handle,
                    aspect_ratio: aspect,
                });
            }
            Err(err) => {
                workshop.error = Some(err);
                continue;
            }
        }
        workshop.error = None;
    }
}

pub(crate) fn gen3d_rebuild_images_list_ui(
    build_scene: Res<State<BuildScene>>,
    mut commands: Commands,
    workshop: Res<Gen3dWorkshop>,
    list_root: Query<Entity, With<Gen3dImagesList>>,
    existing_items: Query<Entity, With<Gen3dImagesListItem>>,
    mut last_paths: Local<Vec<PathBuf>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    let Ok(root) = list_root.single() else {
        return;
    };

    let current_paths: Vec<PathBuf> = workshop.images.iter().map(|i| i.path.clone()).collect();
    if current_paths == *last_paths && !existing_items.is_empty() {
        return;
    }
    *last_paths = current_paths;

    for entity in &existing_items {
        commands.entity(entity).try_despawn();
    }

    commands.entity(root).with_children(|list| {
        if workshop.images.is_empty() {
            return;
        }

        for (idx, img) in workshop
            .images
            .iter()
            .enumerate()
            .take(super::GEN3D_MAX_IMAGES)
        {
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(50.0),
                    height: Val::Px(super::GEN3D_IMAGE_ROW_HEIGHT_PX),
                    min_width: Val::Px(0.0),
                    padding: UiRect::all(Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.55)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                Gen3dImagesListItem,
                Gen3dThumbnailButton::new(idx),
            ))
            .with_children(|row| {
                row.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    ImageNode::new(img.ui_image.clone()).with_mode(NodeImageMode::Stretch),
                ));
            });
        }
    });
}

pub(crate) fn gen3d_thumbnail_button_open_viewer(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut buttons: Query<(&Interaction, &Gen3dThumbnailButton), Changed<Interaction>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    for (interaction, button) in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }
        let index = button.index();
        if index < workshop.images.len() {
            workshop.image_viewer = Some(index);
        }
    }
}

pub(crate) fn gen3d_thumbnail_button_style_on_interaction(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    mut buttons: Query<
        (
            &Interaction,
            &Gen3dThumbnailButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Changed<Interaction>,
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    for (interaction, button, mut bg, mut border) in &mut buttons {
        let selected = workshop.image_viewer == Some(button.index());
        apply_gen3d_thumbnail_style(selected, *interaction, &mut bg, &mut border);
    }
}

pub(crate) fn gen3d_thumbnail_button_style_on_selection(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    mut last_selected: Local<Option<usize>>,
    mut buttons: Query<(
        &Interaction,
        &Gen3dThumbnailButton,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    if *last_selected == workshop.image_viewer {
        return;
    }
    *last_selected = workshop.image_viewer;
    for (interaction, button, mut bg, mut border) in &mut buttons {
        let selected = workshop.image_viewer == Some(button.index());
        apply_gen3d_thumbnail_style(selected, *interaction, &mut bg, &mut border);
    }
}

fn apply_gen3d_thumbnail_style(
    selected: bool,
    interaction: Interaction,
    bg: &mut BackgroundColor,
    border: &mut BorderColor,
) {
    let (mut bg_color, mut border_color) = if selected {
        (
            Color::srgba(0.06, 0.10, 0.07, 0.85),
            Color::srgb(0.25, 0.80, 0.45),
        )
    } else {
        (
            Color::srgba(0.02, 0.02, 0.03, 0.55),
            Color::srgba(0.25, 0.25, 0.30, 0.65),
        )
    };

    match interaction {
        Interaction::Pressed => {
            bg_color = Color::srgba(0.10, 0.18, 0.13, 0.92);
        }
        Interaction::Hovered => {
            bg_color = Color::srgba(0.06, 0.06, 0.075, 0.78);
            if !selected {
                border_color = Color::srgba(0.35, 0.35, 0.40, 0.70);
            }
        }
        Interaction::None => {}
    }

    *bg = BackgroundColor(bg_color);
    *border = BorderColor::all(border_color);
}

pub(crate) fn gen3d_update_thumbnail_tooltip(
    build_scene: Res<State<BuildScene>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    workshop: Res<Gen3dWorkshop>,
    thumbnails: Query<(&Interaction, &Gen3dThumbnailButton)>,
    mut tooltip: Query<(&mut Node, &mut Visibility), With<Gen3dThumbnailTooltipRoot>>,
    mut tooltip_text: Query<&mut Text, With<Gen3dThumbnailTooltipText>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };

    let Ok((mut node, mut vis)) = tooltip.single_mut() else {
        return;
    };
    let Ok(mut text) = tooltip_text.single_mut() else {
        return;
    };

    if workshop.image_viewer.is_some() {
        *vis = Visibility::Hidden;
        return;
    }

    let Some(cursor) = window.cursor_position() else {
        *vis = Visibility::Hidden;
        return;
    };

    let mut hovered: Option<usize> = None;
    for (interaction, btn) in &thumbnails {
        if matches!(*interaction, Interaction::Hovered | Interaction::Pressed) {
            hovered = Some(btn.index());
            break;
        }
    }

    let Some(index) = hovered else {
        *vis = Visibility::Hidden;
        return;
    };
    if index >= workshop.images.len() {
        *vis = Visibility::Hidden;
        return;
    }
    let name = workshop.images[index]
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<unknown>");

    **text = name.into();

    let mut left = cursor.x + 14.0;
    let mut top = cursor.y + 18.0;

    // Rough clamp so it doesn't run off-screen.
    let max_w = window.width().max(1.0);
    let max_h = window.height().max(1.0);
    left = left.clamp(8.0, (max_w - 340.0).max(8.0));
    top = top.clamp(8.0, (max_h - 60.0).max(8.0));

    node.left = Val::Px(left);
    node.top = Val::Px(top);
    *vis = Visibility::Visible;
}

pub(crate) fn gen3d_image_viewer_keyboard_navigation(
    build_scene: Res<State<BuildScene>>,
    input: Res<ButtonInput<KeyCode>>,
    mut workshop: ResMut<Gen3dWorkshop>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    let Some(current) = workshop.image_viewer else {
        return;
    };
    let len = workshop.images.len();
    if len == 0 {
        workshop.image_viewer = None;
        return;
    }

    if input.just_pressed(KeyCode::Escape) {
        workshop.image_viewer = None;
        return;
    }

    if input.just_pressed(KeyCode::ArrowUp) {
        workshop.image_viewer = Some((current + len - 1) % len);
    } else if input.just_pressed(KeyCode::ArrowDown) {
        workshop.image_viewer = Some((current + 1) % len);
    }
}

pub(crate) fn gen3d_image_viewer_click_to_close(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut overlays: Query<&Interaction, (Changed<Interaction>, With<Gen3dImageViewerRoot>)>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    if workshop.image_viewer.is_none() {
        return;
    }
    for interaction in &mut overlays {
        if matches!(*interaction, Interaction::Pressed) {
            workshop.image_viewer = None;
        }
    }
}

pub(crate) fn gen3d_update_image_viewer_ui(
    build_scene: Res<State<BuildScene>>,
    mut commands: Commands,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    workshop: Res<Gen3dWorkshop>,
    existing: Query<Entity, With<Gen3dImageViewerRoot>>,
    mut last_open: Local<Option<usize>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    let desired = workshop.image_viewer;
    let has_overlay = !existing.is_empty();
    if desired == *last_open && has_overlay {
        return;
    }
    *last_open = desired;

    for entity in &existing {
        commands.entity(entity).try_despawn();
    }

    let Some(index) = desired else {
        return;
    };
    if index >= workshop.images.len() {
        return;
    }
    let img = &workshop.images[index];
    let name = img
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<unknown>");

    let (win_w, win_h) = match windows.single() {
        Ok(w) => (w.width().max(1.0), w.height().max(1.0)),
        Err(_) => (1280.0, 720.0),
    };
    let max_w = (win_w * 0.82).clamp(320.0, 1600.0);
    let max_h = (win_h * 0.75).clamp(240.0, 1200.0);
    let aspect = img.aspect_ratio.max(0.05);
    let box_aspect = (max_w / max_h).max(0.05);
    let (fit_w, fit_h) = if aspect >= box_aspect {
        let w = max_w;
        (w, (w / aspect).max(1.0))
    } else {
        let h = max_h;
        ((h * aspect).max(1.0), h)
    };

    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::FlexStart,
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(18.0)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            ZIndex(2500),
            Gen3dImageViewerRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(12.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.94)),
                BorderColor::all(Color::srgb(0.95, 0.85, 0.25)),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new(format!(
                        "Image {}/{}: {name}\n(↑/↓ navigate, Esc/click to close)",
                        index + 1,
                        workshop.images.len()
                    )),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.85, 0.25)),
                    Gen3dImageViewerCaption,
                ));
                panel.spawn((
                    Node {
                        width: Val::Px(fit_w),
                        height: Val::Px(fit_h),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.01, 0.01, 0.015, 0.96)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                    ImageNode::new(img.ui_image.clone()).with_mode(NodeImageMode::Stretch),
                    Gen3dImageViewerImage,
                ));
            });
        });
}

fn is_supported_image_path(path: &PathBuf) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    match ext.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "webp" => true,
        _ => false,
    }
}

fn load_gen3d_ui_image(
    images: &mut Assets<Image>,
    path: &PathBuf,
) -> Result<(Handle<Image>, f32), String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("Unsupported image type: {}", path.display()))?;

    let image = Image::from_buffer(
        &bytes,
        ImageType::Extension(ext),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::linear(),
        bevy::asset::RenderAssetUsages::default(),
    )
    .map_err(|err| format!("Failed to decode {}: {err}", path.display()))?;

    let width = image.texture_descriptor.size.width.max(1) as f32;
    let height = image.texture_descriptor.size.height.max(1) as f32;
    let aspect_ratio = (width / height).clamp(0.05, 20.0);

    Ok((images.add(image), aspect_ratio))
}
