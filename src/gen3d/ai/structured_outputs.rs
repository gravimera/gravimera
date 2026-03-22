use serde_json::json;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dAiJsonSchemaKind {
    AgentStepV1,
    PromptIntentV1,
    PlanV1,
    PlanOpsV1,
    DraftOpsV1,
    ComponentDraftV1,
    ReviewDeltaV1,
    ReviewDeltaNoRegenV1,
    DescriptorMetaV1,
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

fn schema_any_object() -> serde_json::Value {
    json!({
        "type": "object",
        // Some OpenAI-compatible providers validate JSON Schema strictly when `strict=true` and
        // require `additionalProperties=false` for every object schema (including nested ones).
        //
        // The Gen3D agent protocol uses `args` as an arbitrary JSON object whose shape depends on
        // the tool. To keep the protocol flexible while satisfying strict validators, we allow
        // any key via a permissive `patternProperties` entry, and still set
        // `additionalProperties=false` to satisfy providers that require it.
        //
        // Some providers additionally require every object schema to specify both `properties`
        // (possibly empty) and `required` (possibly empty) whenever `type=object`.
        "additionalProperties": false,
        "properties": {},
        "required": [],
        "patternProperties": {
            "^.*$": {}
        }
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

fn schema_agent_step_action_tool_call() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["tool_call"]),
            "call_id": schema_string(),
            "tool_id": schema_string(),
            "args": schema_any_object(),
        },
        "required": ["kind", "call_id", "tool_id", "args"],
    })
}

fn schema_agent_step_action_done() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["done"]),
            "reason": schema_string(),
        },
        "required": ["kind", "reason"],
    })
}

fn schema_agent_step_action() -> serde_json::Value {
    schema_any_of(vec![
        schema_agent_step_action_tool_call(),
        schema_agent_step_action_done(),
    ])
}

fn schema_agent_step() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "version": { "type": "integer", "enum": [1] },
            "status_summary": schema_string(),
            "actions": schema_array_of(schema_agent_step_action()),
        },
        "required": ["version", "status_summary", "actions"],
    })
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
            "alignment_frame",
            schema_nullable(schema_enum(&["join", "child_anchor"])),
        ),
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

fn schema_plan_op() -> serde_json::Value {
    let add_component = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["add_component"]),
            "name": schema_string(),
            "size": schema_vec3(),
            "purpose": schema_string(),
            "modeling_notes": schema_string(),
            "anchors": schema_array_of(schema_anchor()),
            "contacts": schema_array_of(schema_contact()),
            "attach_to": schema_nullable(schema_plan_attachment()),
        },
        "required": ["kind", "name", "size", "purpose", "modeling_notes", "anchors", "contacts", "attach_to"],
    });
    let remove_component = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["remove_component"]),
            "name": schema_string(),
        },
        "required": ["kind", "name"],
    });
    let set_attach_to = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_attach_to"]),
            "component": schema_string(),
            "set_attach_to": schema_nullable(schema_plan_attachment()),
        },
        "required": ["kind", "component", "set_attach_to"],
    });
    let set_anchor = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_anchor"]),
            "component": schema_string(),
            "anchor": schema_anchor(),
        },
        "required": ["kind", "component", "anchor"],
    });
    let set_aim_components = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_aim_components"]),
            "components": schema_array_of(schema_string()),
        },
        "required": ["kind", "components"],
    });
    let set_mobility = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_mobility"]),
            "mobility": schema_mobility(),
        },
        "required": ["kind", "mobility"],
    });
    let set_attack = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_attack"]),
            "attack": schema_nullable(schema_attack()),
        },
        "required": ["kind", "attack"],
    });
    let set_collider = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_collider"]),
            "collider": schema_nullable(schema_collider()),
        },
        "required": ["kind", "collider"],
    });
    let set_attack_muzzle = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_attack_muzzle"]),
            "component": schema_string(),
            "anchor": schema_string(),
        },
        "required": ["kind", "component", "anchor"],
    });
    let set_reuse_groups = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_reuse_groups"]),
            "reuse_groups": schema_array_of(schema_reuse_group()),
        },
        "required": ["kind", "reuse_groups"],
    });

    schema_any_of(vec![
        add_component,
        remove_component,
        set_attach_to,
        set_anchor,
        set_aim_components,
        set_mobility,
        set_attack,
        set_collider,
        set_attack_muzzle,
        set_reuse_groups,
    ])
}

fn schema_plan_ops() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "version": { "type": "integer", "enum": [1] },
            "ops": {
                "type": "array",
                "items": schema_plan_op(),
                "maxItems": 64
            }
        },
        "required": ["version", "ops"],
    })
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
    let tweak_component_resolved_rot_world = schema_object(vec![
        ("kind", schema_enum(&["tweak_component_resolved_rot_world"])),
        ("component_id", schema_string()),
        ("rot", schema_rotation()),
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
        tweak_component_resolved_rot_world,
        tweak_anchor,
        tweak_attachment,
        tweak_contact,
        tweak_mobility,
        tweak_attack,
    ])
}

fn schema_review_delta_action_no_regen() -> serde_json::Value {
    let accept = schema_object(vec![("kind", schema_enum(&["accept"]))]);
    let tooling_feedback = schema_object(vec![
        ("kind", schema_enum(&["tooling_feedback"])),
        ("feedback", schema_tooling_feedback()),
    ]);
    let replan = schema_object(vec![
        ("kind", schema_enum(&["replan"])),
        ("reason", schema_string()),
    ]);
    let tweak_component_transform = schema_object(vec![
        ("kind", schema_enum(&["tweak_component_transform"])),
        ("component_id", schema_string()),
        ("set", schema_nullable(schema_transform_set())),
        ("delta", schema_nullable(schema_transform_delta())),
        ("reason", schema_string()),
    ]);
    let tweak_component_resolved_rot_world = schema_object(vec![
        ("kind", schema_enum(&["tweak_component_resolved_rot_world"])),
        ("component_id", schema_string()),
        ("rot", schema_rotation()),
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
        tweak_component_transform,
        tweak_component_resolved_rot_world,
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
        ("name", schema_string()),
        ("short", schema_string()),
        ("tags", schema_array_of(schema_string())),
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
        ("axis_space", schema_enum(&["join", "child_local"])),
    ]);

    let clip = schema_any_of(vec![clip_loop, clip_once, clip_ping_pong, clip_spin]);

    let slot = schema_object(vec![
        ("channel", schema_string()),
        (
            "driver",
            schema_enum(&[
                "always",
                "move_phase",
                "move_distance",
                "attack_time",
                "action_time",
            ]),
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
            schema_enum(&["author_clips", "regen_geometry_required"]),
        ),
        ("reason", schema_string()),
        ("replace_channels", schema_array_of(schema_string())),
        ("edges", schema_array_of(edge)),
        ("notes", schema_nullable(schema_string())),
    ])
}

fn schema_transform_delta_draft_ops() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "pos": schema_nullable(schema_vec3()),
            "scale": schema_nullable(schema_vec3()),
            "rot_quat_xyzw": schema_nullable(schema_quat_xyzw()),
            "forward": schema_nullable(schema_vec3()),
            "up": schema_nullable(schema_vec3()),
        },
        "required": ["pos", "scale", "rot_quat_xyzw", "forward", "up"],
    })
}

fn schema_primitive_params_draft_ops() -> serde_json::Value {
    let capsule = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["capsule"]),
            "radius": schema_number(),
            "half_length": schema_number(),
        },
        "required": ["kind", "radius", "half_length"],
    });
    let conical_frustum = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["conical_frustum"]),
            "top_radius": schema_number(),
            "bottom_radius": schema_number(),
            "height": schema_number(),
        },
        "required": ["kind", "top_radius", "bottom_radius", "height"],
    });
    let torus = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["torus"]),
            "minor_radius": schema_number(),
            "major_radius": schema_number(),
        },
        "required": ["kind", "minor_radius", "major_radius"],
    });
    schema_any_of(vec![capsule, conical_frustum, torus])
}

fn schema_primitive_spec_draft_ops() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "mesh": schema_string(),
            "params": schema_nullable(schema_primitive_params_draft_ops()),
            "color_rgba": schema_nullable(schema_color_input()),
            "unlit": schema_nullable(schema_bool()),
        },
        "required": ["mesh", "params", "color_rgba", "unlit"],
    })
}

fn schema_animation_clip_draft_ops() -> serde_json::Value {
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
        ("axis_space", schema_enum(&["join", "child_local"])),
    ]);

    schema_any_of(vec![clip_loop, clip_once, clip_ping_pong, clip_spin])
}

fn schema_animation_slot_spec_draft_ops() -> serde_json::Value {
    schema_object(vec![
        (
            "driver",
            schema_enum(&[
                "always",
                "move_phase",
                "move_distance",
                "attack_time",
                "action_time",
            ]),
        ),
        ("speed_scale", schema_number()),
        ("time_offset_units", schema_number()),
        ("clip", schema_animation_clip_draft_ops()),
    ])
}

fn schema_draft_op() -> serde_json::Value {
    let set_anchor_transform = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_anchor_transform"]),
            "component": schema_string(),
            "anchor": schema_string(),
            "set": schema_transform_delta_draft_ops(),
        },
        "required": ["kind", "component", "anchor", "set"],
    });

    let set_attachment_offset = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_attachment_offset"]),
            "child_component": schema_string(),
            "set": schema_transform_delta_draft_ops(),
        },
        "required": ["kind", "child_component", "set"],
    });

    let set_attachment_joint = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["set_attachment_joint"]),
            "child_component": schema_string(),
            "set_joint": schema_nullable(schema_joint()),
        },
        "required": ["kind", "child_component", "set_joint"],
    });

    let update_primitive_part = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["update_primitive_part"]),
            "component": schema_string(),
            "part_id_uuid": schema_string(),
            "set_transform": schema_nullable(schema_transform_delta_draft_ops()),
            "set_primitive": schema_nullable(schema_primitive_spec_draft_ops()),
            "set_render_priority": schema_nullable(schema_integer()),
        },
        "required": ["kind", "component", "part_id_uuid", "set_transform", "set_primitive", "set_render_priority"],
    });

    let add_primitive_part = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["add_primitive_part"]),
            "component": schema_string(),
            "part_id_uuid": schema_string(),
            "primitive": schema_primitive_spec_draft_ops(),
            "transform": schema_transform_delta_draft_ops(),
            "render_priority": schema_nullable(schema_integer()),
        },
        "required": ["kind", "component", "part_id_uuid", "primitive", "transform", "render_priority"],
    });

    let remove_primitive_part = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["remove_primitive_part"]),
            "component": schema_string(),
            "part_id_uuid": schema_string(),
        },
        "required": ["kind", "component", "part_id_uuid"],
    });

    let upsert_animation_slot = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["upsert_animation_slot"]),
            "child_component": schema_string(),
            "channel": schema_string(),
            "slot": schema_animation_slot_spec_draft_ops(),
        },
        "required": ["kind", "child_component", "channel", "slot"],
    });

    let scale_animation_slot_rotation = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["scale_animation_slot_rotation"]),
            "child_component": schema_string(),
            "channel": schema_string(),
            "scale": schema_number(),
        },
        "required": ["kind", "child_component", "channel", "scale"],
    });

    let remove_animation_slot = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": schema_enum(&["remove_animation_slot"]),
            "child_component": schema_string(),
            "channel": schema_string(),
        },
        "required": ["kind", "child_component", "channel"],
    });

    schema_any_of(vec![
        set_anchor_transform,
        set_attachment_offset,
        set_attachment_joint,
        update_primitive_part,
        add_primitive_part,
        remove_primitive_part,
        upsert_animation_slot,
        scale_animation_slot_rotation,
        remove_animation_slot,
    ])
}

fn schema_draft_ops_tool_output() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "version": { "type": "integer", "enum": [1] },
            "ops": schema_array_of(schema_draft_op()),
        },
        "required": ["version", "ops"],
    })
}

fn schema_review_delta_no_regen() -> serde_json::Value {
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
        (
            "actions",
            schema_array_of(schema_review_delta_action_no_regen()),
        ),
        ("summary", schema_nullable(schema_string())),
        ("notes", schema_nullable(schema_string())),
    ])
}

fn schema_prompt_intent() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "version": { "type": "integer", "enum": [1] },
            "requires_attack": schema_bool(),
        },
        "required": ["version", "requires_attack"],
    })
}

pub(super) fn json_schema_spec(kind: Gen3dAiJsonSchemaKind) -> Gen3dAiJsonSchemaSpec {
    match kind {
        Gen3dAiJsonSchemaKind::AgentStepV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_agent_step_v1",
            schema: schema_agent_step(),
        },
        Gen3dAiJsonSchemaKind::PromptIntentV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_prompt_intent_v1",
            schema: schema_prompt_intent(),
        },
        Gen3dAiJsonSchemaKind::PlanV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_plan_v1",
            schema: schema_plan(),
        },
        Gen3dAiJsonSchemaKind::PlanOpsV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_plan_ops_v1",
            schema: schema_plan_ops(),
        },
        Gen3dAiJsonSchemaKind::DraftOpsV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_draft_ops_v1",
            schema: schema_draft_ops_tool_output(),
        },
        Gen3dAiJsonSchemaKind::ComponentDraftV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_component_draft_v1",
            schema: schema_component_draft(),
        },
        Gen3dAiJsonSchemaKind::ReviewDeltaV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_review_delta_v1",
            schema: schema_review_delta(),
        },
        Gen3dAiJsonSchemaKind::ReviewDeltaNoRegenV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_review_delta_no_regen_v1",
            schema: schema_review_delta_no_regen(),
        },
        Gen3dAiJsonSchemaKind::DescriptorMetaV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_descriptor_meta_v1",
            schema: schema_descriptor_meta(),
        },
        Gen3dAiJsonSchemaKind::MotionAuthoringV1 => Gen3dAiJsonSchemaSpec {
            name: "gen3d_motion_authoring_v1",
            schema: schema_motion_authoring(),
        },
    }
}
