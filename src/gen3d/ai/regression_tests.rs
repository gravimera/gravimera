use super::{convert, parse};
use crate::gen3d::state::{
    Gen3dDraft, Gen3dImageRef, Gen3dSideTab, Gen3dSpeedMode, Gen3dStatusLog, Gen3dWorkshop,
};
use crate::object::registry::{
    builtin_object_id, ColliderProfile, ObjectDef, ObjectInteraction, ObjectPartDef, ObjectPartKind,
};
use bevy::prelude::{Quat, Transform, Vec3};
use serde_json::json;

fn id_hex32(id: u128) -> String {
    format!("{:032x}", id)
}

#[test]
fn gen3d_validate_flags_left_right_component_name_mismatch() {
    let root_def = ObjectDef {
        object_id: super::super::gen3d_draft_object_id(),
        label: "gen3d_root".into(),
        size: Vec3::ONE,
        ground_origin_y: None,
        collider: ColliderProfile::None,
        interaction: ObjectInteraction::none(),
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: Vec::new(),
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    };

    let left_def = ObjectDef {
        object_id: builtin_object_id("gravimera/gen3d/component/left_forearm"),
        label: "left_forearm".into(),
        size: Vec3::ONE,
        ground_origin_y: None,
        collider: ColliderProfile::None,
        interaction: ObjectInteraction::none(),
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: Vec::new(),
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    };
    let right_def = ObjectDef {
        object_id: builtin_object_id("gravimera/gen3d/component/right_forearm"),
        label: "right_forearm".into(),
        size: Vec3::ONE,
        ground_origin_y: None,
        collider: ColliderProfile::None,
        interaction: ObjectInteraction::none(),
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: Vec::new(),
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    };

    let draft = Gen3dDraft {
        defs: vec![root_def, left_def, right_def],
    };

    let components = vec![
        super::Gen3dPlannedComponent {
            display_name: "Left Forearm".into(),
            name: "left_forearm".into(),
            purpose: "".into(),
            modeling_notes: "".into(),
            pos: Vec3::new(-0.25, 0.0, 0.0),
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors: Vec::new(),
            contacts: Vec::new(),
            articulation_nodes: Vec::new(),
            root_animations: Vec::new(),
            attach_to: None,
        },
        super::Gen3dPlannedComponent {
            display_name: "Right Forearm".into(),
            name: "right_forearm".into(),
            purpose: "".into(),
            modeling_notes: "".into(),
            pos: Vec3::new(0.25, 0.0, 0.0),
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors: Vec::new(),
            contacts: Vec::new(),
            articulation_nodes: Vec::new(),
            root_animations: Vec::new(),
            attach_to: None,
        },
    ];

    let results = super::build_gen3d_validate_results(&components, &draft);
    let issues = results
        .get("issues")
        .and_then(|v| v.as_array())
        .expect("validate results should include issues[]");

    let mismatches: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            issue.get("kind").and_then(|v| v.as_str())
                == Some("left_right_component_name_mismatch")
        })
        .collect();

    assert_eq!(
        mismatches.len(),
        2,
        "expected 2 left/right mismatch errors, got {}:\n{}",
        mismatches.len(),
        serde_json::to_string_pretty(&results).unwrap_or_default()
    );
}

fn test_workshop() -> Gen3dWorkshop {
    Gen3dWorkshop {
        images: Vec::<Gen3dImageRef>::new(),
        prompt: String::new(),
        prompt_focused: false,
        status: String::new(),
        error: None,
        status_log: Gen3dStatusLog::default(),
        image_viewer: None,
        speed_mode: Gen3dSpeedMode::Level3,
        side_tab: Gen3dSideTab::Status,
        side_panel_open: true,
        prompt_scrollbar_drag: None,
    }
}

fn component_def<'a>(draft: &'a Gen3dDraft, name: &str) -> &'a crate::object::registry::ObjectDef {
    let object_id = builtin_object_id(&format!("gravimera/gen3d/component/{name}"));
    draft
        .defs
        .iter()
        .find(|def| def.object_id == object_id)
        .expect("component def should exist")
}

fn component_animation_slots_for_channel(draft: &Gen3dDraft, name: &str, channel: &str) -> usize {
    component_def(draft, name)
        .parts
        .iter()
        .map(|part| {
            part.animations
                .iter()
                .filter(|slot| slot.channel.as_ref() == channel)
                .count()
        })
        .sum()
}

#[test]
fn gen3d_component_regen_preserves_internal_motion_and_attachment_sync() {
    let plan_text = r#"
    {
      "version": 8,
      "mobility": { "kind": "static" },
      "assembly_notes": "Simple torso and expressive head.",
      "root_component": "torso",
      "components": [
        {
          "name": "torso",
          "purpose": "body",
          "modeling_notes": "simple torso block",
          "size": [1.0, 1.2, 0.8],
          "anchors": [
            { "name": "neck", "pos": [0.0, 0.45, 0.0], "forward": [0,0,1], "up": [0,1,0] }
          ]
        },
        {
          "name": "head",
          "purpose": "expressive head",
          "modeling_notes": "head with face rig handles",
          "size": [0.8, 0.7, 0.7],
          "anchors": [
            { "name": "base", "pos": [0.0, -0.25, 0.0], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "articulation_nodes": [
            { "node_id": "face_root", "pos": [0.0, 0.02, 0.18], "forward": [0,0,1], "up": [0,1,0] },
            { "node_id": "jaw", "parent_node_id": "face_root", "pos": [0.0, -0.10, 0.18], "forward": [0,0,1], "up": [0,1,0] },
            { "node_id": "brow", "parent_node_id": "face_root", "pos": [0.0, 0.12, 0.18], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "attach_to": {
            "parent": "torso",
            "parent_anchor": "neck",
            "child_anchor": "base",
            "offset": { "pos": [0.0, 0.0, -0.005] }
          }
        }
      ]
    }
    "#;

    let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
    let plan_collider = plan.collider.clone();
    let (planned, _notes, defs) =
        convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");

    let mut workshop = test_workshop();
    let mut draft = Gen3dDraft { defs };
    let mut job = super::Gen3dAiJob::default();
    job.plan_hash = "sha256:test".into();
    job.plan_collider = plan_collider;
    job.planned_components = planned;

    let torso_idx = job
        .planned_components
        .iter()
        .position(|component| component.name == "torso")
        .expect("torso should exist");
    let head_idx = job
        .planned_components
        .iter()
        .position(|component| component.name == "head")
        .expect("head should exist");

    let torso_ai = parse::parse_ai_draft_from_text(
        r#"
        {
          "version": 2,
          "collider": null,
          "anchors": [
            { "name": "neck", "pos": [0.0, 0.45, 0.0], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "articulation_nodes": [],
          "parts": [
            { "primitive": "cuboid", "color": [0.3,0.3,0.35,1.0], "pos": [0.0, 0.0, 0.0], "scale": [1.0, 1.2, 0.8] }
          ]
        }
        "#,
    )
    .expect("torso draft should parse");
    let torso_def =
        convert::ai_to_component_def(&job.planned_components[torso_idx], torso_ai, None)
            .expect("torso draft should convert");
    super::component_regen::apply_regenerated_component(
        &mut workshop,
        &mut job,
        &mut draft,
        torso_idx,
        torso_def,
    )
    .expect("torso integration should succeed");

    let head_ai = parse::parse_ai_draft_from_text(
        r#"
        {
          "version": 2,
          "collider": null,
          "anchors": [
            { "name": "base", "pos": [0.0, -0.25, 0.0], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "articulation_nodes": [
            { "node_id": "face_root", "pos": [0.0, 0.02, 0.18], "forward": [0,0,1], "up": [0,1,0], "bind_part_indices": [0] },
            { "node_id": "jaw", "parent_node_id": "face_root", "pos": [0.0, -0.10, 0.18], "forward": [0,0,1], "up": [0,1,0], "bind_part_indices": [1] },
            { "node_id": "brow", "parent_node_id": "face_root", "pos": [0.0, 0.12, 0.18], "forward": [0,0,1], "up": [0,1,0], "bind_part_indices": [2] }
          ],
          "parts": [
            { "primitive": "cuboid", "color": [0.85,0.75,0.65,1.0], "pos": [0.0, 0.05, 0.0], "scale": [0.8, 0.55, 0.7] },
            { "primitive": "cuboid", "color": [0.65,0.35,0.35,1.0], "pos": [0.0, -0.14, 0.18], "scale": [0.42, 0.10, 0.24] },
            { "primitive": "cuboid", "color": [0.25,0.12,0.12,1.0], "pos": [0.0, 0.18, 0.18], "scale": [0.44, 0.06, 0.12] }
          ]
        }
        "#,
    )
    .expect("head draft should parse");
    let head_def = convert::ai_to_component_def(&job.planned_components[head_idx], head_ai, None)
        .expect("head draft should convert");
    super::component_regen::apply_regenerated_component(
        &mut workshop,
        &mut job,
        &mut draft,
        head_idx,
        head_def,
    )
    .expect("head integration should succeed");

    let applies_to = json!({
        "run_id": "",
        "attempt": job.attempt,
        "plan_hash": job.plan_hash.clone(),
        "assembly_rev": job.assembly_rev,
    });
    let smile = parse::parse_ai_motion_authoring_from_text(
        &serde_json::to_string(&json!({
            "version": 1,
            "applies_to": applies_to,
            "decision": "author_clips",
            "replace_channels": ["smile"],
            "targets": [
                {
                    "kind": "articulation_node",
                    "component": "head",
                    "node_id": "face_root",
                    "slots": [{
                        "channel": "smile",
                        "family": "overlay",
                        "driver": "always",
                        "speed_scale": 1.0,
                        "time_offset_units": 0.0,
                        "clip": {
                            "kind": "loop",
                            "duration_units": 1.0,
                            "keyframes": [
                                { "t_units": 0.0, "delta": { "pos": [0.0, 0.0, 0.0] } },
                                { "t_units": 0.5, "delta": { "pos": [0.0, 0.01, 0.0] } },
                                { "t_units": 1.0, "delta": { "pos": [0.0, 0.0, 0.0] } }
                            ]
                        }
                    }]
                },
                {
                    "kind": "articulation_node",
                    "component": "head",
                    "node_id": "jaw",
                    "slots": [{
                        "channel": "smile",
                        "family": "overlay",
                        "driver": "always",
                        "speed_scale": 1.0,
                        "time_offset_units": 0.0,
                        "clip": {
                            "kind": "loop",
                            "duration_units": 1.0,
                            "keyframes": [
                                { "t_units": 0.0, "delta": { "pos": [0.0, 0.0, 0.0] } },
                                { "t_units": 0.5, "delta": { "pos": [0.0, 0.015, 0.0] } },
                                { "t_units": 1.0, "delta": { "pos": [0.0, 0.0, 0.0] } }
                            ]
                        }
                    }]
                }
            ]
        }))
        .expect("smile JSON should serialize"),
    )
    .expect("smile motion should parse");
    super::agent_motion_batch::apply_motion_authoring_for_channel(
        &mut workshop,
        &mut job,
        &mut draft,
        &smile,
        "smile",
    )
    .expect("smile motion should apply");

    let laugh = parse::parse_ai_motion_authoring_from_text(
        &serde_json::to_string(&json!({
            "version": 1,
            "applies_to": {
                "run_id": "",
                "attempt": job.attempt,
                "plan_hash": job.plan_hash.clone(),
                "assembly_rev": job.assembly_rev,
            },
            "decision": "author_clips",
            "replace_channels": ["laugh"],
            "targets": [
                {
                    "kind": "articulation_node",
                    "component": "head",
                    "node_id": "jaw",
                    "slots": [{
                        "channel": "laugh",
                        "family": "overlay",
                        "driver": "always",
                        "speed_scale": 1.0,
                        "time_offset_units": 0.0,
                        "clip": {
                            "kind": "loop",
                            "duration_units": 1.0,
                            "keyframes": [
                                { "t_units": 0.0, "delta": { "pos": [0.0, 0.0, 0.0] } },
                                { "t_units": 0.5, "delta": { "pos": [0.0, -0.02, 0.0] } },
                                { "t_units": 1.0, "delta": { "pos": [0.0, 0.0, 0.0] } }
                            ]
                        }
                    }]
                },
                {
                    "kind": "articulation_node",
                    "component": "head",
                    "node_id": "brow",
                    "slots": [{
                        "channel": "laugh",
                        "family": "overlay",
                        "driver": "always",
                        "speed_scale": 1.0,
                        "time_offset_units": 0.0,
                        "clip": {
                            "kind": "loop",
                            "duration_units": 1.0,
                            "keyframes": [
                                { "t_units": 0.0, "delta": { "pos": [0.0, 0.0, 0.0] } },
                                { "t_units": 0.5, "delta": { "pos": [0.0, 0.02, 0.0] } },
                                { "t_units": 1.0, "delta": { "pos": [0.0, 0.0, 0.0] } }
                            ]
                        }
                    }]
                }
            ]
        }))
        .expect("laugh JSON should serialize"),
    )
    .expect("laugh motion should parse");
    super::agent_motion_batch::apply_motion_authoring_for_channel(
        &mut workshop,
        &mut job,
        &mut draft,
        &laugh,
        "laugh",
    )
    .expect("laugh motion should apply");

    assert!(
        component_animation_slots_for_channel(&draft, "head", "smile") > 0,
        "expected head component to have smile part slots before regen"
    );
    assert!(
        component_animation_slots_for_channel(&draft, "head", "laugh") > 0,
        "expected head component to have laugh part slots before regen"
    );

    job.planned_components[head_idx]
        .attach_to
        .as_mut()
        .expect("head should stay attached")
        .offset
        .translation = Vec3::new(0.0, 0.0, 0.06);

    let regenerated_head_ai = parse::parse_ai_draft_from_text(
        r#"
        {
          "version": 2,
          "collider": null,
          "anchors": [
            { "name": "base", "pos": [0.0, -0.25, 0.0], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "articulation_nodes": [
            { "node_id": "face_root", "pos": [0.0, 0.03, 0.19], "forward": [0,0,1], "up": [0,1,0], "bind_part_indices": [0] },
            { "node_id": "jaw", "parent_node_id": "face_root", "pos": [0.0, -0.11, 0.19], "forward": [0,0,1], "up": [0,1,0], "bind_part_indices": [1] },
            { "node_id": "brow", "parent_node_id": "face_root", "pos": [0.0, 0.13, 0.19], "forward": [0,0,1], "up": [0,1,0], "bind_part_indices": [2] }
          ],
          "parts": [
            { "primitive": "cuboid", "color": [0.87,0.78,0.68,1.0], "pos": [0.0, 0.06, 0.0], "scale": [0.82, 0.58, 0.72] },
            { "primitive": "cuboid", "color": [0.66,0.36,0.36,1.0], "pos": [0.0, -0.15, 0.19], "scale": [0.44, 0.11, 0.24] },
            { "primitive": "cuboid", "color": [0.24,0.10,0.10,1.0], "pos": [0.0, 0.19, 0.19], "scale": [0.46, 0.06, 0.12] }
          ]
        }
        "#,
    )
    .expect("regenerated head draft should parse");
    let regenerated_head =
        convert::ai_to_component_def(&job.planned_components[head_idx], regenerated_head_ai, None)
            .expect("regenerated head draft should convert");
    let replayed = super::component_regen::apply_regenerated_component(
        &mut workshop,
        &mut job,
        &mut draft,
        head_idx,
        regenerated_head,
    )
    .expect("regen integration should succeed");

    assert_eq!(replayed, vec!["laugh".to_string(), "smile".to_string()]);
    assert!(
        component_animation_slots_for_channel(&draft, "head", "smile") > 0,
        "expected smile part slots to survive head regen"
    );
    assert!(
        component_animation_slots_for_channel(&draft, "head", "laugh") > 0,
        "expected laugh part slots to survive head regen"
    );

    let torso_def = component_def(&draft, "torso");
    let head_object_id = builtin_object_id("gravimera/gen3d/component/head");
    let head_ref = torso_def
        .parts
        .iter()
        .find(|part| {
            matches!(part.kind, ObjectPartKind::ObjectRef { object_id } if object_id == head_object_id)
                && part.attachment.is_some()
        })
        .expect("torso should still reference regenerated head");
    assert!(
        (head_ref.transform.translation.z - 0.06).abs() < 1e-5,
        "expected synced head attachment z offset of 0.06, got {:?}",
        head_ref.transform.translation
    );
}

#[test]
fn gen3d_pipeline_warcar_with_cannon_prompt_smoke() {
    // NOTE: This is an offline regression test that exercises the full Gen3D pipeline:
    // plan parse -> plan conversion -> per-component draft parse+convert -> review-delta parse+apply.
    // It intentionally does not call the OpenAI API.

    let plan_text = r#"
    {
      "version": 8,
      "mobility": { "kind": "ground", "max_speed": 7.5 },
      "attack": {
        "kind": "ranged_projectile",
        "cooldown_secs": 0.8,
        "muzzle": { "component": "cannon", "anchor": "muzzle" },
        "projectile": {
          "shape": "sphere",
          "radius": 0.18,
          "color": [1.0, 0.94, 0.75, 1.0],
          "unlit": true,
          "speed": 18.0,
          "ttl_secs": 2.5,
          "damage": 10,
          "obstacle_rule": "bullets_blockers",
          "spawn_energy_impact": false
        }
      },
      "collider": { "kind": "aabb_xz", "half_extents": [2.2, 1.3] },
      "assembly_notes": "A warcar with a cannon as weapon. Keep components symmetric and attach wheels to body anchors.",
      "root_component": "body",
      "components": [
        {
          "name": "body",
          "purpose": "Main chassis",
          "modeling_notes": "Blocky voxel/pixel-art style chassis.",
          "size": [3.6, 1.0, 2.4],
          "anchors": [
            { "name": "wheel_fl", "pos": [ 1.3, -0.35,  0.85], "forward": [0,0,1], "up": [0,1,0] },
            { "name": "wheel_fr", "pos": [-1.3, -0.35,  0.85], "forward": [0,0,1], "up": [0,1,0] },
            { "name": "wheel_bl", "pos": [ 1.3, -0.35, -0.85], "forward": [0,0,1], "up": [0,1,0] },
            { "name": "wheel_br", "pos": [-1.3, -0.35, -0.85], "forward": [0,0,1], "up": [0,1,0] },
            { "name": "cannon_mount", "pos": [0.0, 0.35, 1.0], "forward": [0,0,1], "up": [0,1,0] }
          ]
        },
        {
          "name": "wheel_fl",
          "purpose": "Front-left wheel",
          "modeling_notes": "Cylinder wheel that spins when moving.",
          "size": [0.85, 0.85, 0.35],
          "anchors": [
            { "name": "axle", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "attach_to": {
            "parent": "body",
            "parent_anchor": "wheel_fl",
            "child_anchor": "axle",
            "offset": { "pos": [0,0,0] }
          }
        },
        {
          "name": "wheel_fr",
          "purpose": "Front-right wheel",
          "modeling_notes": "Cylinder wheel that spins when moving.",
          "size": [0.85, 0.85, 0.35],
          "anchors": [
            { "name": "axle", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "attach_to": {
            "parent": "body",
            "parent_anchor": "wheel_fr",
            "child_anchor": "axle",
            "offset": { "pos": [0,0,0] }
          }
        },
        {
          "name": "wheel_bl",
          "purpose": "Back-left wheel",
          "modeling_notes": "Cylinder wheel that spins when moving.",
          "size": [0.85, 0.85, 0.35],
          "anchors": [
            { "name": "axle", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "attach_to": {
            "parent": "body",
            "parent_anchor": "wheel_bl",
            "child_anchor": "axle",
            "offset": { "pos": [0,0,0] }
          }
        },
        {
          "name": "wheel_br",
          "purpose": "Back-right wheel",
          "modeling_notes": "Cylinder wheel that spins when moving.",
          "size": [0.85, 0.85, 0.35],
          "anchors": [
            { "name": "axle", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "attach_to": {
            "parent": "body",
            "parent_anchor": "wheel_br",
            "child_anchor": "axle",
            "offset": { "pos": [0,0,0] }
          }
        },
        {
          "name": "cannon",
          "purpose": "Cannon weapon",
          "modeling_notes": "Short barrel on top of chassis; keep it chunky.",
          "size": [1.2, 0.6, 1.0],
          "anchors": [
            { "name": "mount", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] },
            { "name": "muzzle", "pos": [0.0, 0.0, 0.65], "forward": [0,0,1], "up": [0,1,0] }
          ],
          "attach_to": {
            "parent": "body",
            "parent_anchor": "cannon_mount",
            "child_anchor": "mount",
            "offset": { "pos": [0,0,0] }
          }
        }
      ]
    }
    "#;

    let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
    let plan_collider = plan.collider.clone();
    let (mut planned, _notes, defs) =
        convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");
    assert_eq!(planned.len(), 6);

    let mut draft = Gen3dDraft { defs };

    let component_drafts: [(&str, &str); 6] = [
        (
            "body",
            r#"
            {
              "version": 2,
              "anchors": [
                { "name": "wheel_fl", "pos": [ 1.3, -0.35,  0.85], "forward": [0,0,1], "up": [0,1,0] },
                { "name": "wheel_fr", "pos": [-1.3, -0.35,  0.85], "forward": [0,0,1], "up": [0,1,0] },
                { "name": "wheel_bl", "pos": [ 1.3, -0.35, -0.85], "forward": [0,0,1], "up": [0,1,0] },
                { "name": "wheel_br", "pos": [-1.3, -0.35, -0.85], "forward": [0,0,1], "up": [0,1,0] },
                { "name": "cannon_mount", "pos": [0.0, 0.35, 1.0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "parts": [
                { "primitive": "cuboid", "color": [0.24,0.28,0.33,1.0], "pos": [0.0, 0.0, 0.0], "scale": [3.6, 0.8, 2.4] },
                { "primitive": "cuboid", "color": [0.18,0.20,0.24,1.0], "pos": [0.0, 0.35, -0.4], "scale": [2.2, 0.35, 1.4] }
              ]
            }
            "#,
        ),
        (
            "wheel_fl",
            r#"
            {
              "version": 2,
              "anchors": [
                { "name": "axle", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "parts": [
                {
                  "primitive": "cylinder",
                  "color": [0.12,0.12,0.13,1.0],
                  "pos": [0.0,0.0,0.0],
                  "forward": [1,0,0],
                  "up": [0,1,0],
                  "scale": [0.35, 0.85, 0.35]
                }
              ]
            }
            "#,
        ),
        (
            "wheel_fr",
            r#"
            {
              "version": 2,
              "anchors": [
                { "name": "axle", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "parts": [
                {
                  "primitive": "cylinder",
                  "color": [0.12,0.12,0.13,1.0],
                  "pos": [0.0,0.0,0.0],
                  "forward": [1,0,0],
                  "up": [0,1,0],
                  "scale": [0.35, 0.85, 0.35]
                }
              ]
            }
            "#,
        ),
        (
            "wheel_bl",
            r#"
            {
              "version": 2,
              "anchors": [
                { "name": "axle", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "parts": [
                {
                  "primitive": "cylinder",
                  "color": [0.12,0.12,0.13,1.0],
                  "pos": [0.0,0.0,0.0],
                  "forward": [1,0,0],
                  "up": [0,1,0],
                  "scale": [0.35, 0.85, 0.35]
                }
              ]
            }
            "#,
        ),
        (
            "wheel_br",
            r#"
            {
              "version": 2,
              "anchors": [
                { "name": "axle", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "parts": [
                {
                  "primitive": "cylinder",
                  "color": [0.12,0.12,0.13,1.0],
                  "pos": [0.0,0.0,0.0],
                  "forward": [1,0,0],
                  "up": [0,1,0],
                  "scale": [0.35, 0.85, 0.35]
                }
              ]
            }
            "#,
        ),
        (
            "cannon",
            r#"
            {
              "version": 2,
              "anchors": [
                { "name": "mount", "pos": [0,0,0], "forward": [0,0,1], "up": [0,1,0] },
                { "name": "muzzle", "pos": [0.0, 0.0, 0.65], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "parts": [
                { "primitive": "cuboid", "color": [0.20,0.22,0.26,1.0], "pos": [0.0, 0.0, 0.0], "scale": [1.1, 0.35, 0.55] },
                { "primitive": "cylinder", "color": [0.10,0.10,0.12,1.0], "pos": [0.0, 0.0, 0.45], "forward": [0,0,1], "up": [0,1,0], "scale": [0.18, 0.70, 0.18] }
              ]
            }
            "#,
        ),
    ];

    for (name, draft_text) in component_drafts {
        let component_idx = planned
            .iter()
            .position(|c| c.name == name)
            .unwrap_or(usize::MAX);
        assert!(
            component_idx < planned.len(),
            "component `{name}` should exist"
        );

        let ai = parse::parse_ai_draft_from_text(draft_text).expect("draft should parse");
        let def = convert::ai_to_component_def(&planned[component_idx], ai, None)
            .expect("draft should convert");

        let object_id = def.def.object_id;
        planned[component_idx].actual_size = Some(def.def.size);
        planned[component_idx].anchors = def.def.anchors.clone();
        planned[component_idx].articulation_nodes = def.articulation_nodes.clone();

        if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == object_id) {
            let preserved_refs: Vec<ObjectPartDef> = existing
                .parts
                .iter()
                .filter(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. }))
                .cloned()
                .collect();
            let mut new_def = def.def;
            new_def.parts.extend(preserved_refs);
            *existing = new_def;
        } else {
            draft.defs.push(def.def);
        }

        if let Some(root_idx) = planned.iter().position(|c| c.attach_to.is_none()) {
            convert::resolve_planned_component_transforms(&mut planned, root_idx)
                .expect("planned transforms should resolve");
        }
        convert::update_root_def_from_planned_components(&planned, &plan_collider, &mut draft);
    }

    assert!(
        draft.root_def().is_some(),
        "draft should contain gen3d_draft root def"
    );
    assert!(
        draft.total_non_projectile_primitive_parts() >= 6,
        "draft should have primitive parts"
    );
    assert_eq!(
        draft.component_count(),
        1,
        "draft root should reference exactly the root component"
    );

    let body_id = crate::object::registry::builtin_object_id("gravimera/gen3d/component/body");
    let cannon_id = crate::object::registry::builtin_object_id("gravimera/gen3d/component/cannon");

    let delta_value = serde_json::json!({
      "version": 1,
      "applies_to": {"run_id":"test","attempt":0,"plan_hash":"sha256:test","assembly_rev":0},
      "actions": [
        {
          "kind": "tweak_attachment",
          "component_id": id_hex32(cannon_id),
          "set": {
            "parent_component_id": id_hex32(body_id),
            "parent_anchor": "cannon_mount",
            "child_anchor": "mount",
            "offset": {
              "pos": [0.0, 0.0, 0.0],
              "rot_frame": "join",
              "rot_quat_xyzw": [0.0, 0.0, 0.0, 1.0]
            }
          },
          "reason": "ensure cannon attachment offset supports explicit quaternion rotations"
        }
      ]
    });
    let delta_text =
        serde_json::to_string_pretty(&delta_value).expect("delta JSON should serialize");
    let delta = parse::parse_ai_review_delta_from_text(&delta_text).expect("delta should parse");
    let apply = convert::apply_ai_review_delta_actions(
        delta,
        &mut planned,
        &plan_collider,
        &mut draft,
        None,
    )
    .expect("delta should apply");
    assert!(
        apply.had_actions,
        "review delta should apply at least one action"
    );
}

#[test]
fn gen3d_review_delta_prompt_includes_join_axes_and_offset_pos() {
    let scene_graph_summary = json!({
        "version": 1,
        "root": {
            "size": [3.0, 2.0, 5.0],
            "mobility": { "kind": "ground", "max_speed": 10.0 },
            "attack": { "kind": "ranged_projectile", "cooldown_secs": 1.0, "damage": 10, "anim_window_secs": 0.2 },
            "collider": { "kind": "aabb_xz", "half_extents": [1.5, 2.5] }
        },
        "components": [
            {
                "name": "chassis",
                "component_id_uuid": "00000000-0000-0000-0000-000000000001",
                "planned_size": [2.0, 1.0, 4.0],
                "actual_size": [2.0, 1.0, 4.0],
                "resolved_transform": { "pos": [0.0, 0.0, 0.0], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] },
                "anchors": [{ "name": "turret_mount" }],
                "attach_to": null
            },
            {
                "name": "turret_base",
                "component_id_uuid": "00000000-0000-0000-0000-000000000002",
                "planned_size": [1.0, 1.0, 1.0],
                "actual_size": [1.0, 1.0, 1.0],
                "resolved_transform": { "pos": [0.1, 0.7, 0.6], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] },
                "anchors": [{ "name": "mount_bottom" }],
                "attach_to": {
                    "parent_component_name": "chassis",
                    "parent_component_id_uuid": "00000000-0000-0000-0000-000000000001",
                    "parent_anchor": "turret_mount",
                    "child_anchor": "mount_bottom",
                    "join_forward_world": [0.0, 1.0, 0.0],
                    "join_up_world": [0.0, 0.0, 1.0],
                    "join_x_world": [-1.0, 0.0, 0.0],
                    "offset": { "pos": [0.0, 0.0, -0.02] },
                    "joint": { "kind": "hinge", "axis_join": [0.0, 1.0, 0.0], "limits_degrees": [-90.0, 90.0] }
                }
            }
        ]
    });

    let smoke_results = json!({
        "version": 1,
        "has_images": false,
        "issues": [],
        "ok": true,
    });

    let text = super::prompts::build_gen3d_review_delta_user_text(
        "deadbeef-dead-beef-dead-beefdeadbeef",
        0,
        "sha256:abc",
        42,
        "A warcar with a cannon as weapon",
        None,
        &scene_graph_summary,
        &smoke_results,
        1,
        2,
    );

    assert!(
        text.contains("offset.pos(join_frame)="),
        "missing offset.pos(join_frame)= in prompt:\n{text}"
    );
    assert!(
        text.contains("join_x_world=")
            && text.contains("join_up_world=")
            && text.contains("join_forward_world="),
        "missing join axes in prompt:\n{text}"
    );
    assert!(
        text.contains("joint=hinge") && text.contains("limits_deg="),
        "missing joint kind/limits in prompt:\n{text}"
    );
}

#[test]
fn gen3d_scene_graph_summary_includes_joint_kind() {
    use crate::object::registry::AnchorDef;
    use std::borrow::Cow;

    let components = vec![
        super::Gen3dPlannedComponent {
            display_name: "Root".into(),
            name: "root".into(),
            purpose: "".into(),
            modeling_notes: "".into(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors: vec![AnchorDef {
                name: Cow::Borrowed("child_mount"),
                transform: Transform::IDENTITY,
            }],
            contacts: vec![],
            articulation_nodes: vec![],
            root_animations: vec![],
            attach_to: None,
        },
        super::Gen3dPlannedComponent {
            display_name: "Child".into(),
            name: "child".into(),
            purpose: "".into(),
            modeling_notes: "".into(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors: vec![AnchorDef {
                name: Cow::Borrowed("base"),
                transform: Transform::IDENTITY,
            }],
            contacts: vec![],
            articulation_nodes: vec![],
            root_animations: vec![],
            attach_to: Some(super::Gen3dPlannedAttachment {
                parent: "root".into(),
                parent_anchor: "child_mount".into(),
                child_anchor: "base".into(),
                offset: Transform::IDENTITY,
                fallback_basis: Transform::IDENTITY,
                joint: Some(super::AiJointJson {
                    kind: super::AiJointKindJson::Free,
                    axis_join: None,
                    limits_degrees: None,
                    swing_limits_degrees: None,
                    twist_limits_degrees: None,
                }),
                animations: vec![],
            }),
        },
    ];

    let summary = super::build_gen3d_scene_graph_summary(
        "deadbeef-dead-beef-dead-beefdeadbeef",
        0,
        0,
        "sha256:abc",
        1,
        &components,
        &Gen3dDraft::default(),
    );

    assert_eq!(
        summary
            .get("components_total")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        2
    );
    assert_eq!(
        summary
            .get("attachments_total")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        1
    );
    let edges = summary
        .get("attachment_edges")
        .and_then(|v| v.as_array())
        .expect("attachment_edges array");
    assert_eq!(edges.len(), 1);
    assert_eq!(
        edges[0].get("child").and_then(|v| v.as_str()),
        Some("child")
    );
    assert_eq!(
        edges[0].get("parent").and_then(|v| v.as_str()),
        Some("root")
    );

    let comps = summary
        .get("components")
        .and_then(|v| v.as_array())
        .expect("components array");
    let child = comps
        .iter()
        .find(|c| c.get("name").and_then(|v| v.as_str()) == Some("child"))
        .expect("child component");
    let kind = child
        .get("attach_to")
        .and_then(|a| a.get("joint"))
        .and_then(|j| j.get("kind"))
        .and_then(|v| v.as_str())
        .expect("joint kind");
    assert_eq!(kind, "free");
}

#[test]
fn gen3d_rejects_plan_animations() {
    let plan_text = r#"
    {
      "version": 8,
      "mobility": {"kind":"static"},
      "assembly_notes": "plan animations are disallowed",
      "root_component": "head",
      "components": [
        {
          "name": "head",
          "purpose": "head",
          "modeling_notes": "",
          "size": [0.30, 0.30, 0.30],
          "anchors": [
            { "name": "hair_mount", "pos": [0.0, 0.15, 0.0], "forward": [0.0, 1.0, 0.0], "up": [0.0, 0.0, 1.0] }
          ]
        },
        {
          "name": "hair",
          "purpose": "hair",
          "modeling_notes": "",
          "size": [0.30, 0.32, 0.26],
          "anchors": [
            { "name": "scalp_mount", "pos": [0.0, -0.14, 0.0], "forward": [0.0, 1.0, 0.0], "up": [0.0, 0.0, 1.0] }
          ],
          "attach_to": {
            "parent": "head",
            "parent_anchor": "hair_mount",
            "child_anchor": "scalp_mount",
            "offset": { "pos": [0.0, 0.0, -0.005] },
            "joint": { "kind": "fixed" },
            "animations": {
              "idle": {
                "driver": "always",
                "clip": {
                  "kind": "loop",
                  "duration_secs": 1.0,
                  "keyframes": [
                    {
                      "time_secs": 0.0,
                      "delta": { "forward": [1.0, 0.0, 0.0], "up": [0.0, 1.0, 0.0], "rot_frame": "join" }
                    }
                  ]
                }
              }
            }
          }
        }
      ]
    }
    "#;

    assert!(
        parse::parse_ai_plan_from_text(plan_text).is_err(),
        "expected attach_to.animations to be rejected"
    );
}
