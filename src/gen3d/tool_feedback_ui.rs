use bevy::prelude::*;

use crate::config::AppConfig;
use crate::types::GameMode;

use super::state::*;
use super::tool_feedback::{gen3d_cache_base_dir, Gen3dToolFeedbackHistory};

pub(crate) fn gen3d_side_tab_buttons(
    mode: Res<State<GameMode>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut buttons: Query<
        (&Interaction, &Gen3dSideTabButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }

    for (interaction, button, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                workshop.side_tab = button.tab();
                if matches!(workshop.side_tab, Gen3dSideTab::ToolFeedback) {
                    workshop.tool_feedback_unread = false;
                }
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
    mode: Res<State<GameMode>>,
    workshop: Res<Gen3dWorkshop>,
    mut panels: ParamSet<(
        Query<(&mut Node, &mut Visibility), With<Gen3dStatusPanelRoot>>,
        Query<(&mut Node, &mut Visibility), With<Gen3dToolFeedbackPanelRoot>>,
    )>,
    mut buttons: Query<(&Gen3dSideTabButton, &Interaction, &mut BackgroundColor)>,
    mut texts: Query<(&Gen3dSideTabButtonText, &mut Text)>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
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
        let active = matches!(workshop.side_tab, Gen3dSideTab::ToolFeedback);
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
            Gen3dSideTab::ToolFeedback => {
                if workshop.tool_feedback_unread {
                    "Tool Feedback*".to_string()
                } else {
                    "Tool Feedback".to_string()
                }
            }
        };
        **text = label.into();
    }
}

pub(crate) fn gen3d_update_tool_feedback_text(
    mode: Res<State<GameMode>>,
    config: Res<AppConfig>,
    workshop: Res<Gen3dWorkshop>,
    history: Res<Gen3dToolFeedbackHistory>,
    mut texts: Query<&mut Text, With<Gen3dToolFeedbackText>>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }

    if !matches!(workshop.side_tab, Gen3dSideTab::ToolFeedback) {
        return;
    }

    let run_id = history.last_run_id().map(|s| s.to_string());

    let mut out = String::new();
    if history.entries.is_empty() {
        out.push_str("No tool feedback yet.\n\n");
        out.push_str(
            "When the Gen3D reviewer wants new tools or improvements, it will add entries here.\n",
        );
    } else if let Some(run_id) = run_id.as_deref() {
        let entries: Vec<_> = history.entries_for_run(run_id).collect();
        out.push_str(&format!(
            "Last run: {run_id}\nEntries: {}\n\n",
            entries.len()
        ));
        if entries.is_empty() {
            out.push_str("No feedback entries for the last run.\n");
        } else {
            for entry in entries {
                out.push_str(&format!(
                    "[{}] {} (attempt {:?} / pass {:?})\n{}\n\n",
                    entry.priority.to_uppercase(),
                    entry.title.trim(),
                    entry.attempt.unwrap_or(0),
                    entry.pass.unwrap_or(0),
                    entry.summary.trim()
                ));
            }
        }

        let run_dir = gen3d_cache_base_dir(&config).join(run_id);
        out.push_str("Files:\n");
        out.push_str(&format!(
            "- {}\n",
            run_dir.join("tool_feedback.jsonl").display()
        ));
        out.push_str(&format!(
            "- {}\n",
            gen3d_cache_base_dir(&config)
                .join("tool_feedback_history.jsonl")
                .display()
        ));
    }

    for mut text in &mut texts {
        **text = out.clone().into();
    }
}

pub(crate) fn gen3d_tool_feedback_scroll_wheel(
    mode: Res<State<GameMode>>,
    workshop: Res<Gen3dWorkshop>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<bevy::input::mouse::MouseWheel>,
    mut panels: Query<
        (&ComputedNode, &UiGlobalTransform, &mut ScrollPosition),
        With<Gen3dToolFeedbackScrollPanel>,
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }
    if !matches!(workshop.side_tab, Gen3dSideTab::ToolFeedback) {
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

pub(crate) fn gen3d_update_tool_feedback_scrollbar_ui(
    mode: Res<State<GameMode>>,
    workshop: Res<Gen3dWorkshop>,
    panels: Query<&ComputedNode, With<Gen3dToolFeedbackScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<Gen3dToolFeedbackScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<Gen3dToolFeedbackScrollbarThumb>>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }
    if !matches!(workshop.side_tab, Gen3dSideTab::ToolFeedback) {
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

    *track_vis = Visibility::Visible;

    let max_scroll = (content_h - viewport_h).max(1.0);
    let scroll_y = panel.scroll_position.y.clamp(0.0, max_scroll);

    let min_thumb_h = 14.0;
    let thumb_h = (track_h * (viewport_h / content_h)).clamp(min_thumb_h, track_h);
    let thumb_top = (track_h - thumb_h) * (scroll_y / max_scroll);

    thumb.top = Val::Px(thumb_top);
    thumb.height = Val::Px(thumb_h);
}

pub(crate) fn gen3d_copy_tool_feedback_buttons(
    mode: Res<State<GameMode>>,
    config: Res<AppConfig>,
    mut workshop: ResMut<Gen3dWorkshop>,
    history: Res<Gen3dToolFeedbackHistory>,
    mut buttons: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            Option<&Gen3dCopyFeedbackCodexButton>,
            Option<&Gen3dCopyFeedbackJsonButton>,
        ),
        Changed<Interaction>,
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }

    let run_id = history.last_run_id().map(|s| s.to_string());

    for (interaction, mut bg, codex, json) in &mut buttons {
        let is_codex = codex.is_some();
        let is_json = json.is_some();

        if !is_codex && !is_json {
            continue;
        }

        match *interaction {
            Interaction::Pressed => {
                if is_codex {
                    *bg = BackgroundColor(Color::srgba(0.10, 0.14, 0.22, 0.90));
                    if let Some(run_id) = run_id.as_deref() {
                        let payload = build_codex_clipboard_payload(&config, &history, run_id);
                        if crate::clipboard::write_text(&payload) {
                            workshop.status = format!("Copied tool feedback for run {run_id}.");
                            workshop.error = None;
                        } else {
                            workshop.error = Some("Failed to copy to clipboard.".into());
                        }
                    } else {
                        workshop.error = Some("No tool feedback entries to copy yet.".into());
                    }
                } else {
                    *bg = BackgroundColor(Color::srgba(0.10, 0.12, 0.14, 0.88));
                    if let Some(run_id) = run_id.as_deref() {
                        let entries: Vec<_> = history.entries_for_run(run_id).cloned().collect();
                        match serde_json::to_string_pretty(&entries) {
                            Ok(json) => {
                                if crate::clipboard::write_text(&json) {
                                    workshop.status =
                                        format!("Copied JSON tool feedback for run {run_id}.");
                                    workshop.error = None;
                                } else {
                                    workshop.error =
                                        Some("Failed to copy JSON to clipboard.".into());
                                }
                            }
                            Err(err) => {
                                workshop.error = Some(format!("Failed to serialize JSON: {err}"));
                            }
                        }
                    } else {
                        workshop.error = Some("No tool feedback entries to copy yet.".into());
                    }
                }
            }
            Interaction::Hovered => {
                if is_codex {
                    *bg = BackgroundColor(Color::srgba(0.08, 0.12, 0.20, 0.86));
                } else {
                    *bg = BackgroundColor(Color::srgba(0.08, 0.10, 0.12, 0.82));
                }
            }
            Interaction::None => {
                if is_codex {
                    *bg = BackgroundColor(Color::srgba(0.06, 0.10, 0.16, 0.80));
                } else {
                    *bg = BackgroundColor(Color::srgba(0.08, 0.10, 0.12, 0.78));
                }
            }
        }
    }
}

fn build_codex_clipboard_payload(
    config: &AppConfig,
    history: &Gen3dToolFeedbackHistory,
    run_id: &str,
) -> String {
    let repo_path = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<unknown>".into());
    let git = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| o.status.success().then_some(o))
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "<unknown>".into());

    let entries: Vec<_> = history.entries_for_run(run_id).collect();
    let base_dir = gen3d_cache_base_dir(config);
    let run_dir = base_dir.join(run_id);

    let mut out = String::new();
    out.push_str("# Gravimera Gen3D tooling feedback (last run)\n");
    out.push_str(&format!("Repo: {repo_path}\n"));
    out.push_str(&format!("Git: {git}\n"));
    out.push_str(&format!("Run: {run_id}\n\n"));

    out.push_str("Summary:\n");
    if entries.is_empty() {
        out.push_str("- (no tool feedback entries for this run)\n");
    } else {
        for e in &entries {
            out.push_str(&format!(
                "- [{}] {} — {} (attempt_{}/pass_{})\n",
                e.priority.to_uppercase(),
                e.title.trim(),
                e.summary.trim(),
                e.attempt.unwrap_or(0),
                e.pass.unwrap_or(0)
            ));
        }
    }

    out.push_str("\nFiles:\n");
    out.push_str(&format!(
        "- {}\n",
        run_dir.join("tool_feedback.jsonl").display()
    ));
    out.push_str(&format!(
        "- {}\n",
        base_dir.join("tool_feedback_history.jsonl").display()
    ));
    out.push_str(&format!(
        "- {}\n",
        run_dir.join("attempt_*/pass_*/gen3d_run.log").display()
    ));
    out.push_str(&format!(
        "- {}\n",
        run_dir.join("attempt_*/pass_*/gravimera.log").display()
    ));
    out.push_str(&format!(
        "- {}\n",
        run_dir.join("attempt_*/pass_*/review_*.png").display()
    ));

    out
}
