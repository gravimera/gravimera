use serde_json::json;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dAiJsonSchemaKind {
    PlanV1,
    ComponentDraftV1,
    ReviewDeltaV1,
    DescriptorMetaV1,
    MotionRolesV1,
    MotionAuthoringV1,
}

pub(super) struct Gen3dAiJsonSchemaSpec {
    pub(super) name: &'static str,
    pub(super) schema: serde_json::Value,
}

fn schema_null() -> serde_json::Value {
    json!({ "type": "null" })
}

fn schema_number() -> serde_json::Value {
    json!({ "type": "number" })
}

fn schema_integer() -> serde_json::Value {
    json!({ "type": "integer" })
}

fn schema_string() -> serde_json::Value {
    json!({ "type": "string" })
}

fn schema_bool() -> serde_json::Value {
    json!({ "type": "boolean" })
}

fn schema_enum(values: &[&str]) -> serde_json::Value {
    json!({ "enum": values })
}

fn schema_any_of(options: Vec<serde_json::Value>) -> serde_json::Value {
    json!({ "anyOf": options })
}

fn schema_nullable(inner: serde_json::Value) -> serde_json::Value {
    schema_any_of(vec![inner, schema_null()])
}

fn schema_array_of(items: serde_json::Value) -> serde_json::Value {
    json!({
        "type": "array",
        "items": items
    })
}

fn schema_fixed_num_array(len: usize) -> serde_json::Value {
    json!({
        "type": "array",
        "items": schema_number(),
        "minItems": len,
        "maxItems": len
    })
}

fn schema_vec2() -> serde_json::Value {
    schema_fixed_num_array(2)
}

fn schema_vec3() -> serde_json::Value {
    schema_fixed_num_array(3)
}

fn schema_quat_xyzw() -> serde_json::Value {
    schema_fixed_num_array(4)
}

fn schema_object(properties: Vec<(&str, serde_json::Value)>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let mut required: Vec<String> = Vec::new();
    for (key, schema) in properties {
        map.insert(key.to_string(), schema);
        required.push(key.to_string());
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": map,
        "required": required,
    })
}

fn schema_anchor() -> serde_json::Value {
    schema_object(vec![
        ("name", schema_string()),
        ("pos", schema_vec3()),
        ("forward", schema_vec3()),
        ("up", schema_vec3()),
    ])
}

fn schema_collider() -> serde_json::Value {
    let none = schema_object(vec![("kind", schema_enum(&["none"]))]);
    let circle = schema_object(vec![
        ("kind", schema_enum(&["circle_xz"])),
        ("radius", schema_number()),
    ]);
    let aabb = schema_object(vec![
        ("kind", schema_enum(&["aabb_xz"])),
        ("half_extents", schema_nullable(schema_vec2())),
        ("min", schema_nullable(schema_vec2())),
        ("max", schema_nullable(schema_vec2())),
    ]);
    schema_any_of(vec![none, circle, aabb])
}

fn schema_mobility() -> serde_json::Value {
    let static_kind = schema_object(vec![("kind", schema_enum(&["static"]))]);
    let ground = schema_object(vec![
        ("kind", schema_enum(&["ground"])),
        ("max_speed", schema_number()),
    ]);
    let air = schema_object(vec![
        ("kind", schema_enum(&["air"])),
        ("max_speed", schema_number()),
    ]);
    schema_any_of(vec![static_kind, ground, air])
}

fn schema_color_input() -> serde_json::Value {
    schema_fixed_num_array(4)
}

fn schema_anchor_ref() -> serde_json::Value {
    schema_object(vec![
        ("component", schema_string()),
        ("anchor", schema_string()),
    ])
}

fn schema_projectile_spec() -> serde_json::Value {
    schema_object(vec![
        (
            "shape",
            schema_enum(&["sphere", "capsule", "cuboid", "cylinder"]),
        ),
        ("radius", schema_nullable(schema_number())),
        ("length", schema_nullable(schema_number())),
        ("size", schema_nullable(schema_vec3())),
        ("color", schema_color_input()),
        ("unlit", schema_bool()),
        ("speed", schema_number()),
        ("ttl_secs", schema_number()),
        ("damage", schema_integer()),
        (
            "obstacle_rule",
            schema_nullable(schema_enum(&["bullets_blockers", "laser_blockers"])),
        ),
        ("spawn_energy_impact", schema_bool()),
    ])
}

fn schema_attack() -> serde_json::Value {
    let none = schema_object(vec![("kind", schema_enum(&["none"]))]);
    let melee = schema_object(vec![
        ("kind", schema_enum(&["melee"])),
        ("cooldown_secs", schema_nullable(schema_number())),
        ("damage", schema_nullable(schema_integer())),
        ("range", schema_nullable(schema_number())),
        ("radius", schema_nullable(schema_number())),
        ("arc_degrees", schema_nullable(schema_number())),
    ]);
    let ranged = schema_object(vec![
        ("kind", schema_enum(&["ranged_projectile"])),
        ("cooldown_secs", schema_nullable(schema_number())),
        ("muzzle", schema_nullable(schema_anchor_ref())),
        ("projectile", schema_nullable(schema_projectile_spec())),
    ]);
    schema_any_of(vec![none, melee, ranged])
}

fn schema_aim() -> serde_json::Value {
    schema_object(vec![
        ("max_yaw_delta_degrees", schema_nullable(schema_number())),
        ("components", schema_array_of(schema_string())),
    ])
}

fn schema_attachment_offset() -> serde_json::Value {
    schema_object(vec![
        ("pos", schema_vec3()),
        ("forward", schema_nullable(schema_vec3())),
        ("up", schema_nullable(schema_vec3())),
        ("rot_frame", schema_enum(&["join", "parent"])),
        ("rot_quat_xyzw", schema_nullable(schema_quat_xyzw())),
        ("scale", schema_nullable(schema_vec3())),
    ])
}

fn schema_joint() -> serde_json::Value {
    schema_object(vec![
        ("kind", schema_enum(&["fixed", "hinge", "ball", "free"])),
        ("axis_join", schema_nullable(schema_vec3())),
        ("limits_degrees", schema_nullable(schema_vec2())),
        ("swing_limits_degrees", schema_nullable(schema_vec2())),
        ("twist_limits_degrees", schema_nullable(schema_vec2())),
    ])
}

fn schema_contact_stance() -> serde_json::Value {
    schema_object(vec![
        ("phase_01", schema_number()),
        ("duty_factor_01", schema_number()),
    ])
}

fn schema_contact() -> serde_json::Value {
    schema_object(vec![
        ("name", schema_string()),
        ("anchor", schema_string()),
        ("kind", schema_enum(&["ground"])),
        ("stance", schema_nullable(schema_contact_stance())),
    ])
}

fn schema_plan_attachment() -> serde_json::Value {
    schema_object(vec![
        ("parent", schema_string()),
        ("parent_anchor", schema_string()),
        ("child_anchor", schema_string()),
        ("offset", schema_nullable(schema_attachment_offset())),
        ("joint", schema_nullable(schema_joint())),
    ])
}

fn schema_plan_component() -> serde_json::Value {
    schema_object(vec![
        ("name", schema_string()),
        ("purpose", schema_string()),
        ("modeling_notes", schema_string()),
        ("size", schema_vec3()),
        ("anchors", schema_array_of(schema_anchor())),
        ("contacts", schema_array_of(schema_contact())),
        ("attach_to", schema_nullable(schema_plan_attachment())),
    ])
}

fn schema_reuse_group() -> serde_json::Value {
    schema_object(vec![
        (
            "kind",
            schema_enum(&[
                "component",
                "copy_component",
                "subtree",
                "copy_component_subtree",
            ]),
        ),
        ("source", schema_string()),
        ("targets", schema_array_of(schema_string())),
        ("alignment", schema_enum(&["rotation", "mirror_mount_x"])),
        (
            "mode",
            schema_nullable(schema_enum(&["detached", "linked"])),
        ),
        (
            "anchors",
            schema_nullable(schema_enum(&[
                "preserve_interfaces",
                "preserve_target",
                "copy_source",
            ])),
        ),
    ])
}

fn schema_rig() -> serde_json::Value {
    schema_object(vec![("move_cycle_m", schema_nullable(schema_number()))])
}

fn schema_plan() -> serde_json::Value {
    schema_object(vec![
        ("version", schema_integer()),
        ("rig", schema_nullable(schema_rig())),
        ("mobility", schema_mobility()),
        ("attack", schema_nullable(schema_attack())),
        ("aim", schema_nullable(schema_aim())),
        ("collider", schema_nullable(schema_collider())),
        ("assembly_notes", schema_string()),
        ("root_component", schema_nullable(schema_string())),
        ("reuse_groups", schema_array_of(schema_reuse_group())),
        ("components", schema_array_of(schema_plan_component())),
    ])
}

fn schema_primitive_params() -> serde_json::Value {
    let capsule = schema_object(vec![
        ("kind", schema_enum(&["capsule"])),
        ("half_length", schema_number()),
        ("radius", schema_number()),
    ]);
    let conical_frustum = schema_object(vec![
        ("kind", schema_enum(&["conical_frustum"])),
        ("top_radius", schema_number()),
        ("bottom_radius", schema_number()),
        ("height", schema_number()),
    ]);
    let torus = schema_object(vec![
        ("kind", schema_enum(&["torus"])),
        ("minor_radius", schema_number()),
        ("major_radius", schema_number()),
    ]);
    schema_any_of(vec![capsule, conical_frustum, torus])
}

fn schema_part() -> serde_json::Value {
    schema_object(vec![
        (
            "primitive",
            schema_enum(&["cuboid", "cone", "cylinder", "sphere"]),
        ),
        ("params", schema_nullable(schema_primitive_params())),
        ("color", schema_nullable(schema_fixed_num_array(4))),
        ("render_priority", schema_nullable(schema_integer())),
        ("pos", schema_vec3()),
        ("forward", schema_nullable(schema_vec3())),
        ("up", schema_nullable(schema_vec3())),
        ("scale", schema_vec3()),
    ])
}

fn schema_component_draft() -> serde_json::Value {
    schema_object(vec![
        ("version", schema_integer()),
        ("collider", schema_nullable(schema_collider())),
        ("anchors", schema_array_of(schema_anchor())),
        ("parts", schema_array_of(schema_part())),
    ])
}

fn schema_rotation() -> serde_json::Value {
    let basis = schema_object(vec![("forward", schema_vec3()), ("up", schema_vec3())]);
    let quat = schema_object(vec![("quat_xyzw", schema_quat_xyzw())]);
    schema_any_of(vec![basis, quat])
}

fn schema_transform_set() -> serde_json::Value {
    schema_object(vec![
        ("pos", schema_nullable(schema_vec3())),
        ("scale", schema_nullable(schema_vec3())),
        ("rot", schema_nullable(schema_rotation())),
    ])
}

fn schema_transform_delta() -> serde_json::Value {
    schema_object(vec![
        ("pos", schema_nullable(schema_vec3())),
        ("scale", schema_nullable(schema_vec3())),
        ("rot_quat_xyzw", schema_nullable(schema_quat_xyzw())),
    ])
}

fn schema_anchor_set() -> serde_json::Value {
    schema_object(vec![
        ("pos", schema_nullable(schema_vec3())),
        ("forward", schema_nullable(schema_vec3())),
        ("up", schema_nullable(schema_vec3())),
    ])
}

fn schema_anchor_delta() -> serde_json::Value {
    schema_object(vec![
        ("pos", schema_nullable(schema_vec3())),
        ("rot_quat_xyzw", schema_nullable(schema_quat_xyzw())),
    ])
}

fn schema_attachment_set() -> serde_json::Value {
    schema_object(vec![
        ("parent_component_id", schema_string()),
        ("parent_anchor", schema_string()),
        ("child_anchor", schema_string()),
        ("offset", schema_nullable(schema_attachment_offset())),
    ])
}

fn schema_tooling_feedback() -> serde_json::Value {
    schema_object(vec![
        ("version", schema_integer()),
        ("priority", schema_string()),
        ("title", schema_string()),
        ("summary", schema_string()),
        ("details", schema_string()),
    ])
}

fn schema_review_delta_action() -> serde_json::Value {
    let accept = schema_object(vec![("kind", schema_enum(&["accept"]))]);
    let tooling_feedback = schema_object(vec![
        ("kind", schema_enum(&["tooling_feedback"])),
        ("feedback", schema_tooling_feedback()),
    ]);
    let replan = schema_object(vec![
        ("kind", schema_enum(&["replan"])),
        ("reason", schema_string()),
    ]);
    let regen_component = schema_object(vec![
        ("kind", schema_enum(&["regen_component"])),
        ("component_id", schema_string()),
        ("updated_modeling_notes", schema_string()),
        ("reason", schema_string()),
    ]);
    let tweak_component_transform = schema_object(vec![
        ("kind", schema_enum(&["tweak_component_transform"])),
        ("component_id", schema_string()),
        ("set", schema_nullable(schema_transform_set())),
        ("delta", schema_nullable(schema_transform_delta())),
        ("reason", schema_string()),
    ]);
    let tweak_anchor = schema_object(vec![
        ("kind", schema_enum(&["tweak_anchor"])),
        ("component_id", schema_string()),
        ("anchor_name", schema_string()),
        ("set", schema_nullable(schema_anchor_set())),
        ("delta", schema_nullable(schema_anchor_delta())),
        ("reason", schema_string()),
    ]);
    let tweak_attachment = schema_object(vec![
        ("kind", schema_enum(&["tweak_attachment"])),
        ("component_id", schema_string()),
        ("set", schema_attachment_set()),
        ("reason", schema_string()),
    ]);
    let tweak_contact = schema_object(vec![
        ("kind", schema_enum(&["tweak_contact"])),
        ("component_id", schema_string()),
        ("contact_name", schema_string()),
        ("stance", schema_nullable(schema_contact_stance())),
        ("reason", schema_string()),
    ]);
    let tweak_mobility = schema_object(vec![
        ("kind", schema_enum(&["tweak_mobility"])),
        ("mobility", schema_mobility()),
        ("reason", schema_string()),
    ]);
    let tweak_attack = schema_object(vec![
        ("kind", schema_enum(&["tweak_attack"])),
        ("attack", schema_attack()),
        ("reason", schema_string()),
    ]);

    schema_any_of(vec![
        accept,
        tooling_feedback,
        replan,
        regen_component,
        tweak_component_transform,
        tweak_anchor,
        tweak_attachment,
        tweak_contact,
        tweak_mobility,
        tweak_attack,
    ])
}

fn schema_review_delta() -> serde_json::Value {
    schema_object(vec![
        ("version", schema_integer()),
        (
            "applies_to",
            schema_object(vec![
                ("run_id", schema_string()),
                ("attempt", schema_integer()),
                ("plan_hash", schema_string()),
                ("assembly_rev", schema_integer()),
            ]),
        ),
        ("actions", schema_array_of(schema_review_delta_action())),
        ("summary", schema_nullable(schema_string())),
        ("notes", schema_nullable(schema_string())),
    ])
}

fn schema_descriptor_meta() -> serde_json::Value {
    schema_object(vec![
        ("version", schema_integer()),
        ("short", schema_string()),
        ("tags", schema_array_of(schema_string())),
    ])
}

fn schema_motion_roles() -> serde_json::Value {
    let applies_to = schema_object(vec![
        ("run_id", schema_string()),
        ("attempt", schema_integer()),
        ("plan_hash", schema_string()),
        ("assembly_rev", schema_integer()),
    ]);

    let effector = schema_object(vec![
        ("component", schema_string()),
        (
            "role",
            schema_enum(&[
                "leg",
                "wheel",
                "arm",
                "head",
                "ear",
                "tail",
                "wing",
                "propeller",
                "rotor",
            ]),
        ),
        ("phase_group", schema_nullable(schema_integer())),
        ("spin_axis_local", schema_nullable(schema_vec3())),
    ]);

    schema_object(vec![
        ("version", schema_integer()),
        ("applies_to", applies_to),
        ("move_effectors", schema_array_of(effector)),
        ("notes", schema_nullable(schema_string())),
    ])
}

fn schema_motion_authoring() -> serde_json::Value {
    let applies_to = schema_object(vec![
        ("run_id", schema_string()),
        ("attempt", schema_integer()),
        ("plan_hash", schema_string()),
        ("assembly_rev", schema_integer()),
    ]);

    let delta = schema_object(vec![
        ("pos", schema_nullable(schema_vec3())),
        ("rot_quat_xyzw", schema_nullable(schema_quat_xyzw())),
        ("scale", schema_nullable(schema_vec3())),
    ]);

    let keyframe = schema_object(vec![("t_units", schema_number()), ("delta", delta)]);

    let clip_loop = schema_object(vec![
        ("kind", schema_enum(&["loop"])),
        ("duration_units", schema_number()),
        ("keyframes", schema_array_of(keyframe.clone())),
    ]);
    let clip_once = schema_object(vec![
        ("kind", schema_enum(&["once"])),
        ("duration_units", schema_number()),
        ("keyframes", schema_array_of(keyframe.clone())),
    ]);
    let clip_ping_pong = schema_object(vec![
        ("kind", schema_enum(&["ping_pong"])),
        ("duration_units", schema_number()),
        ("keyframes", schema_array_of(keyframe)),
    ]);
    let clip_spin = schema_object(vec![
        ("kind", schema_enum(&["spin"])),
        ("axis", schema_vec3()),
        ("radians_per_unit", schema_number()),
    ]);

    let clip = schema_any_of(vec![clip_loop, clip_once, clip_ping_pong, clip_spin]);

    let slot = schema_object(vec![
        ("channel", schema_string()),
        (
            "driver",
            schema_enum(&["always", "move_phase", "move_distance", "attack_time"]),
        ),
        ("speed_scale", schema_number()),
        ("time_offset_units", schema_number()),
        ("clip", clip),
    ]);

    let edge = schema_object(vec![
        ("component", schema_string()),
        ("slots", schema_array_of(slot)),
    ]);

    schema_object(vec![
        ("version", schema_integer()),
        ("applies_to", applies_to),
        (
            "decision",
            schema_enum(&["runtime_ok", "author_clips", "regen_geometry_required"]),
        ),
        ("reason", schema_string()),
        ("replace_channels", schema_array_of(schema_string())),
        ("edges", schema_array_of(edge)),
        ("notes", schema_nullable(schema_string())),
    ])
}

pub(super) fn json_schema_spec(kind: Gen3dAiJsonSchemaKind) -> Gen3dAiJsonSchemaSpec {
    match kind {
        Gen3dAiJsonSchemaKind::PlanV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_plan_v1",
            schema: schema_plan(),
        },
        Gen3dAiJsonSchemaKind::ComponentDraftV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_component_draft_v1",
            schema: schema_component_draft(),
        },
        Gen3dAiJsonSchemaKind::ReviewDeltaV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_review_delta_v1",
            schema: schema_review_delta(),
        },
        Gen3dAiJsonSchemaKind::DescriptorMetaV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_descriptor_meta_v1",
            schema: schema_descriptor_meta(),
        },
        Gen3dAiJsonSchemaKind::MotionRolesV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_motion_roles_v1",
            schema: schema_motion_roles(),
        },
        Gen3dAiJsonSchemaKind::MotionAuthoringV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_motion_authoring_v1",
            schema: schema_motion_authoring(),
        },
    }
}
