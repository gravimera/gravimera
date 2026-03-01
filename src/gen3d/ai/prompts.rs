use bevy::prelude::*;

use super::Gen3dPlannedComponent;
use crate::gen3d::state::Gen3dSpeedMode;

use crate::gen3d::{GEN3D_DEFAULT_STYLE_PROMPT, GEN3D_MAX_COMPONENTS, GEN3D_MAX_PARTS};

pub(super) fn build_gen3d_effective_user_prompt(raw_prompt: &str) -> String {
    let trimmed = raw_prompt.trim();
    let mut out = String::new();
    out.push_str(
        "If photos are provided, choose the main object in the photos (ignore the background).\n\
         If no photos are provided, infer the object solely from the user notes.\n",
    );
    out.push_str(
        "Default style (use this unless the user explicitly requests a different style): ",
    );
    out.push_str(GEN3D_DEFAULT_STYLE_PROMPT);
    out.push('\n');
    out.push_str(
        "Modeling priorities:\n\
         - Reproduce the BASIC STRUCTURE and proportions first (silhouette and main volumes).\n\
         - Components must be explainable (each one corresponds to a meaningful/structural sub-part).\n\
         - Avoid small decorative details unless they are critical to the object's identity.\n",
    );
    out.push_str(
        "Orientation conventions:\n\
         - Treat the object's main/front facing direction as +Z (forward).\n\
         - Treat the object's right direction as +X.\n\
         - Keep the overall object centered near the origin.\n",
    );

    if trimmed.is_empty() {
        out.push_str("User notes: (none)\n");
    } else {
        out.push_str(
            "User notes (may include style overrides; if so, prefer the user over the default):\n",
        );
        out.push_str(trimmed);
        out.push('\n');
    }

    out
}

pub(super) fn build_gen3d_plan_user_text(
    raw_prompt: &str,
    has_images: bool,
    speed: Gen3dSpeedMode,
) -> String {
    let mut out = String::new();
    out.push_str("Step 1 (planning): Split the object into multiple components.\n");
    out.push_str(
        "Each component must have a stable unique name, a purpose, and an approximate `size`.\n\
         Components are assembled via a TREE of anchor attachments (no absolute placement).\n\
         Each component must define named `anchors` (pos + forward/up in component-local space).\n\
         Anchor NAMES must be stable: the next step will ask you to output the same anchor names again with precise frames.\n\
         Every non-root component must define `attach_to` (parent component + anchor names).\n\
         Also decide the object's `mobility` (static vs ground vs air).\n\
         Use `attach_to.offset.pos` to explicitly encode overlap/inset/outset at joins (the engine will not auto-adjust placement).\n\
         Avoid z-fighting at joins: do NOT make parent/child faces flush and coplanar; add a small epsilon offset along the attachment direction (e.g. `attach_to.offset.pos[2]` ~= 0.005m).\n\
         Define attachment anchors as JOIN frames (each expressed in its OWN component-local coordinates):\n\
          - Set `parent_anchor.forward` (+Z) to point from the parent toward the child (attachment direction) in the PARENT component's local axes.\n\
          - Set `child_anchor.forward` (+Z) and `child_anchor.up` (+Y) in the CHILD component's local axes so the child can rotate into the parent's join frame.\n\
            They do NOT need to numerically equal the parent's vectors.\n\
            Example: if a chain link is modeled along the child's local +Z axis, use `forward=[0,0,1]` and `up=[0,1,0]` for its joint anchors.\n\
          - Do NOT make the join frames 180° opposed (that flips the child). If you need a flip, encode it via `attach_to.offset` rotation.\n\
            - Whenever you author an `attach_to.offset` rotation (`offset.forward`/`offset.up` or `offset.rot_quat_xyzw`), you MUST include `offset.rot_frame` explicitly (`\"join\"` or `\"parent\"`).\n\
         Then `attach_to.offset.pos[2]` becomes a reliable in/out control along the attachment direction.\n",
    );
    out.push_str(
        "Placement sanity check (ignore rotation): estimate `child_origin ~= parent_anchor.pos + attach_to.offset.pos - child_anchor.pos`.\n\
Ensure this puts the component where it should visually sit.\n\
If you intend a thin surface layer, keep its size small along the attachment direction; if you intend a wrap-around layer, anchor it near the parent's center (or split it into front/back layers).\n",
    );
    out.push_str(&format!("Speed mode: {}.\n", speed.label()));
    out.push_str(&format!(
        "Hard cap: at most {} components.\n",
        super::max_components_for_speed(speed)
    ));
    out.push_str("Goal: Use a reasonable number of components and ensure they fit/align well.\n");
    if speed.wants_component_interaction() {
        out.push_str(
            "Component interaction requirements:\n\
             - Plan shared dimensions and alignment rules (flush faces, symmetry, attachment points).\n\
             - Use `assembly_notes` to describe key constraints so component generation stays consistent.\n\
             - Keep the assembled object coherent near the origin (avoid scattering components).\n",
        );
    }
    if !has_images {
        out.push_str("No photos are provided for this run.\n");
    }
    out.push_str(&build_gen3d_effective_user_prompt(raw_prompt));
    out
}

pub(super) fn build_gen3d_plan_user_text_with_hints(
    raw_prompt: &str,
    has_images: bool,
    speed: Gen3dSpeedMode,
    style_hint: Option<&str>,
    required_component_names: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("Step 1 (planning): Split the object into multiple components.\n");
    out.push_str(
        "Each component must have a stable unique name, a purpose, and an approximate `size`.\n\
         Components are assembled via a TREE of anchor attachments (no absolute placement).\n\
         Each component must define named `anchors` (pos + forward/up in component-local space).\n\
         Anchor NAMES must be stable: the next step will ask you to output the same anchor names again with precise frames.\n\
         Every non-root component must define `attach_to` (parent component + anchor names).\n\
         Also decide the object's `mobility` (static vs ground vs air).\n\
         Use `attach_to.offset.pos` to explicitly encode overlap/inset/outset at joins (the engine will not auto-adjust placement).\n\
         Avoid z-fighting at joins: do NOT make parent/child faces flush and coplanar; add a small epsilon offset along the attachment direction (e.g. `attach_to.offset.pos[2]` ~= 0.005m).\n\
         Define attachment anchors as JOIN frames (each expressed in its OWN component-local coordinates):\n\
          - Set `parent_anchor.forward` (+Z) to point from the parent toward the child (attachment direction) in the PARENT component's local axes.\n\
          - Set `child_anchor.forward` (+Z) and `child_anchor.up` (+Y) in the CHILD component's local axes so the child can rotate into the parent's join frame.\n\
            They do NOT need to numerically equal the parent's vectors.\n\
            Example: if a chain link is modeled along the child's local +Z axis, use `forward=[0,0,1]` and `up=[0,1,0]` for its joint anchors.\n\
          - Do NOT make the join frames 180° opposed (that flips the child). If you need a flip, encode it via `attach_to.offset` rotation.\n\
            - Whenever you author an `attach_to.offset` rotation (`offset.forward`/`offset.up` or `offset.rot_quat_xyzw`), you MUST include `offset.rot_frame` explicitly (`\"join\"` or `\"parent\"`).\n\
         Then `attach_to.offset.pos[2]` becomes a reliable in/out control along the attachment direction.\n",
    );
    out.push_str(
        "Placement sanity check (ignore rotation): estimate `child_origin ~= parent_anchor.pos + attach_to.offset.pos - child_anchor.pos`.\n\
Ensure this puts the component where it should visually sit.\n\
If you intend a thin surface layer, keep its size small along the attachment direction; if you intend a wrap-around layer, anchor it near the parent's center (or split it into front/back layers).\n",
    );
    out.push_str(&format!("Speed mode: {}.\n", speed.label()));
    out.push_str(&format!(
        "Hard cap: at most {} components.\n",
        super::max_components_for_speed(speed)
    ));
    out.push_str("Goal: Use a reasonable number of components and ensure they fit/align well.\n");
    if speed.wants_component_interaction() {
        out.push_str(
            "Component interaction requirements:\n\
             - Plan shared dimensions and alignment rules (flush faces, symmetry, attachment points).\n\
             - Use `assembly_notes` to describe key constraints so component generation stays consistent.\n\
             - Keep the assembled object coherent near the origin (avoid scattering components).\n",
        );
    }

    if !required_component_names.is_empty() {
        out.push_str(
            "Component naming constraint: You MUST use EXACTLY these component names (and ONLY these) in the plan.\n\
If you feel you need extra pieces, merge them into these components instead of adding new components.\n\
Required component names (in order):\n",
        );
        for name in required_component_names {
            let n = name.trim();
            if n.is_empty() {
                continue;
            }
            out.push_str(&format!("- {n}\n"));
        }
    }

    if let Some(style) = style_hint.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        out.push_str("Additional style preference (use this unless the user notes forbid it): ");
        out.push_str(style);
        out.push('\n');
    }

    if !has_images {
        out.push_str("No photos are provided for this run.\n");
    }
    out.push_str(&build_gen3d_effective_user_prompt(raw_prompt));
    out
}

pub(super) fn build_gen3d_component_user_text(
    raw_prompt: &str,
    has_images: bool,
    speed: Gen3dSpeedMode,
    assembly_notes: &str,
    components: &[Gen3dPlannedComponent],
    component_index: usize,
) -> String {
    let total = components.len().max(1);
    let idx = component_index.min(total - 1);
    let component = &components[idx];
    let budget = (GEN3D_MAX_PARTS / total).clamp(16, 256);

    let mut out = String::new();
    out.push_str("Step 2 (component generation): Generate ONLY the requested component.\n");
    out.push_str("Do not generate other components in this step.\n");
    out.push_str(
        "IMPORTANT: Output component-local coordinates.\n\
         The engine assembles components by aligning named anchors (tree attachments).\n\
         Do NOT bake any assembly transforms into the component's parts.\n\
         Size contract: `target_size` is an axis-aligned AABB size in THIS component's local axes (+X right, +Y up, +Z forward).\n\
         Your generated primitive parts should produce a component whose local AABB size is close to `target_size` on each axis (do not swap/permutate axes).\n\
         If the AABB axes appear permuted relative to `target_size`, the engine will reject the draft and request regeneration.\n\
         Attachment frame rule: parent/child anchors describe the SAME join frame but in different component-local coordinates; do NOT force them to have identical numeric vectors.\n\
         Avoid 180° opposition (that flips the child). If you need a flip, encode it via `attach_to.offset` rotation in the PLAN (not by reversing anchors).\n\
         Convention: the component center must be at local [0,0,0].\n\
         The engine will not auto-adjust placement.\n\
             Avoid z-fighting: do NOT place two primitives so any of their planar faces are coplanar AND overlap in area.\n\
              - If you layer a detail on a surface (trim/panel/hatch/window band), offset it slightly OUTWARD along the surface normal (a small epsilon, e.g. ~0.005m).\n\
              - For concentric capped cylinders/frustums, do not give multiple primitives the exact same cap plane; shorten/offset inner ones slightly.\n\
              - Output base solids first, then surface details later.\n\
              - Optional: set `render_priority` (small integer) to help the renderer break any remaining depth ties.\n\
                Use `0` for base solids, `1` for surface details, `2` for very thin decals; keep values small (|render_priority| <= 3).\n",
        );
    out.push_str(&format!(
        "Component parts budget: try to stay within ~{budget} primitives.\n"
    ));
    out.push_str(
        "Color requirement: every part MUST include `color` as [r,g,b,a] in 0..1 (do not omit; do not use material names).\n",
    );
    if speed.wants_component_interaction() {
        out.push_str(
            "Interaction reminder: This component must fit with the other components using the plan's anchor attachments.\n\
             Use consistent shared dimensions and place anchors precisely where attachments should occur.\n",
        );
    }
    if !has_images {
        out.push_str("No photos are provided for this run.\n");
    }
    out.push_str(&build_gen3d_effective_user_prompt(raw_prompt));
    out.push('\n');

    out.push_str("Plan context (compact; for scale and attachments):\n");
    if let Some(root) = components.iter().find(|c| c.attach_to.is_none()) {
        let root_size = root.actual_size.unwrap_or(root.planned_size);
        out.push_str(&format!(
            "- root: {} size≈[{:.3},{:.3},{:.3}]\n",
            root.name, root_size.x, root_size.y, root_size.z
        ));
    }
    if let Some(att) = component.attach_to.as_ref() {
        if let Some(parent) = components.iter().find(|c| c.name == att.parent) {
            let parent_size = parent.actual_size.unwrap_or(parent.planned_size);
            out.push_str(&format!(
                "- parent: {} size≈[{:.3},{:.3},{:.3}] parent_anchor={} child_anchor={}\n",
                parent.name,
                parent_size.x,
                parent_size.y,
                parent_size.z,
                att.parent_anchor,
                att.child_anchor
            ));
        } else {
            out.push_str(&format!(
                "- parent: {} parent_anchor={} child_anchor={}\n",
                att.parent, att.parent_anchor, att.child_anchor
            ));
        }
    }
    if !components.is_empty() {
        let mut names: Vec<&str> = components.iter().map(|c| c.name.as_str()).collect();
        // Keep this short; the engine keeps the full plan on disk if needed.
        if names.len() > 24 {
            names.truncate(24);
        }
        out.push_str(&format!("- components: {:?}\n", names));
    }

    out.push_str("\nComponent to generate:\n");
    out.push_str(&format!(
        "- name: {}\n- purpose: {}\n- modeling_notes: {}\n- target_size: [{:.3},{:.3},{:.3}]\n",
        component.name,
        component.purpose,
        component.modeling_notes,
        component.planned_size.x,
        component.planned_size.y,
        component.planned_size.z,
    ));

    if let Some(att) = component.attach_to.as_ref() {
        let forward = att.offset.rotation * Vec3::Z;
        let up = att.offset.rotation * Vec3::Y;
        out.push_str(&format!(
            "- attach_to: parent={}, parent_anchor={}, child_anchor={}\n- attach_offset_pos: [{:.3},{:.3},{:.3}]\n- attach_offset_forward: [{:.2},{:.2},{:.2}]\n- attach_offset_up: [{:.2},{:.2},{:.2}]\n",
            att.parent,
            att.parent_anchor,
            att.child_anchor,
            att.offset.translation.x,
            att.offset.translation.y,
            att.offset.translation.z,
            forward.x,
            forward.y,
            forward.z,
            up.x,
            up.y,
            up.z,
        ));

        let child_anchor_rot = if att.child_anchor == "origin" {
            Some(Quat::IDENTITY)
        } else {
            component
                .anchors
                .iter()
                .find(|a| a.name.as_ref() == att.child_anchor)
                .map(|a| a.transform.rotation)
        };
        if let Some(child_anchor_rot) = child_anchor_rot {
            let mut spin_axes: Vec<(String, Vec3)> = Vec::new();
            for slot in att.animations.iter() {
                if let crate::object::registry::PartAnimationDef::Spin { axis, .. } =
                    &slot.spec.clip
                {
                    let axis_component_local = (child_anchor_rot
                        * (att.offset.rotation.inverse() * *axis))
                        .normalize_or_zero();
                    if axis_component_local.length_squared() > 1e-6 {
                        spin_axes.push((slot.channel.as_ref().to_string(), axis_component_local));
                    }
                }
            }
            if !spin_axes.is_empty() {
                out.push_str("- spin_axis_hint (component-local):\n");
                for (channel, axis) in spin_axes {
                    out.push_str(&format!(
                        "  - channel={} axis≈[{:.2},{:.2},{:.2}]\n",
                        channel, axis.x, axis.y, axis.z
                    ));
                }
                out.push_str("  - Note: the engine will NOT auto-align spinner geometry to the spin axis; rotate your primitives explicitly if you want non-tumbling spin.\n");
            }
        }
    } else {
        out.push_str("- attach_to: (none; this is the root component)\n");
    }

    out.push_str("\nRequired anchors for this component (MUST include all in output JSON):\n");
    for a in component.anchors.iter() {
        let forward = a.transform.rotation * Vec3::Z;
        let up = a.transform.rotation * Vec3::Y;
        let pos = a.transform.translation;
        out.push_str(&format!(
            "- {}: approx pos=[{:.3},{:.3},{:.3}], forward=[{:.2},{:.2},{:.2}], up=[{:.2},{:.2},{:.2}]\n",
            a.name,
            pos.x,
            pos.y,
            pos.z,
            forward.x,
            forward.y,
            forward.z,
            up.x,
            up.y,
            up.z
        ));
    }

    let mut children: Vec<&Gen3dPlannedComponent> = components
        .iter()
        .filter(|c| {
            c.attach_to
                .as_ref()
                .is_some_and(|att| att.parent == component.name)
        })
        .collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));
    if !children.is_empty() {
        out.push_str("\nChildren that attach to this component (for context):\n");
        for child in children {
            if let Some(att) = child.attach_to.as_ref() {
                out.push_str(&format!(
                    "- {}: parent_anchor={} (on this component), child_anchor={} (on child)\n",
                    child.name, att.parent_anchor, att.child_anchor
                ));
            }
        }
    }

    let notes = assembly_notes.trim();
    if !notes.is_empty() {
        out.push_str("\nAssembly notes:\n");
        out.push_str(notes);
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen3d_plan_prompt_mentions_attachment_placement_sanity_check() {
        let prompt = build_gen3d_plan_user_text("test", false, Gen3dSpeedMode::Level3);
        assert!(prompt.contains("Placement sanity check"));
        let prompt =
            build_gen3d_plan_user_text_with_hints("test", false, Gen3dSpeedMode::Level3, None, &[]);
        assert!(prompt.contains("Placement sanity check"));
    }
}

pub(super) fn build_gen3d_plan_system_instructions() -> String {
    format!(
        "You are a 3D modeling assistant.\n\
         Return STRICT JSON for a component assembly plan.\n\n\
         Coordinate system:\n\
         - +Y is up, +X is right, +Z is forward.\n\
         - Orientations are given as direction vectors (no Euler angles).\n\
         - Each component has its own LOCAL axes: +X right, +Y up, +Z forward.\n\n\
         Assembly model:\n\
         - Components are assembled by aligning named ANCHORS in a TREE of attachments.\n\
         - There is NO absolute component placement in this plan.\n\n\
         Mobility decision:\n\
         - Decide whether the object should be movable.\n\
         - Output `mobility` with `kind`:\n\
           - `static` (buildings/props)\n\
           - `ground` (walk/run/drive; provide `max_speed`)\n\
           - `air` (fly; provide `max_speed`)\n\
         - Strong guideline:\n\
           - If the prompt describes a creature/character/vehicle (e.g. horse, soldier, knight, goblin, car, truck, tank), prefer `ground`.\n\
           - Use `static` primarily for buildings/props (e.g. chair, table, lamp, door) unless the prompt explicitly says it is a controllable unit.\n\n\
         Combat decision (optional):\n\
         - Decide whether the object should be able to attack.\n\
         - Most buildings/props are NOT attack-capable.\n\
         - Most characters/units with weapons are attack-capable.\n\
         - Output `attack` ONLY if the object is movable AND attack-capable.\n\
         - `attack.kind`:\n\
           - `none`\n\
           - `melee` (swing/hit in close range)\n\
           - `ranged_projectile` (shoot projectiles)\n\
         - If you output `attack`, include ALL required fields for that kind (do not output partial attack objects).\n\
         - `melee` fields: `cooldown_secs`, `damage`, `range`, `radius`, `arc_degrees`.\n\
         - `ranged_projectile` fields:\n\
           - `cooldown_secs`\n\
           - `muzzle`: {{ `component`, `anchor` }}\n\
           - `projectile`: {{\n\
               `shape`: `sphere` | `capsule` | `cylinder` | `cuboid`\n\
               (`radius`/`length`/`size` depending on shape)\n\
               `color`: [r,g,b,a]  (REQUIRED; 0..1)\n\
               `unlit`: bool\n\
               `speed`: number\n\
               `ttl_secs`: number\n\
               `damage`: integer\n\
               `obstacle_rule`: `bullets_blockers` | `laser_blockers` (optional)\n\
               `spawn_energy_impact`: bool (optional)\n\
             }}\n\
         - For `ranged_projectile`, you MUST provide the `muzzle` anchor reference (component+anchor) and define that anchor in the referenced component.\n\n\
         Aim / attention constraints (optional):\n\
         - If the object is attack-capable (or has a head/weapon/turret that should track targets), you may output `aim`.\n\
         - This defines the MAX allowed yaw difference between the unit BODY direction (movement facing) and its ATTENTION direction (weapon/head facing).\n\
         - `aim.max_yaw_delta_degrees`: number or null.\n\
           - null (or omitted) means unlimited (can aim freely 360 degrees).\n\
           - smaller values (e.g. 45..120) limit how far the weapon/head can turn away from the body.\n\
         - `aim.components`: list of component NAMES that should yaw with the attention direction (e.g. `head`, `turret`, `weapon`, `cannon`).\n\
         - If `aim` is omitted for a ranged unit, the engine will aim the muzzle component by default (or its parent component when the muzzle is a nested helper).\n\n\
         Animation policy:\n\
         - Gen3D currently generates STATIC models only.\n\
         - Do NOT author any per-edge animation clips in the plan (there is no `attach_to.animations`).\n\n\
         Anchor definition:\n\
         - An anchor is a named coordinate frame inside a component.\n\
         - `pos` is anchor origin in component-local coordinates.\n\
         - `forward` is the anchor's +Z direction.\n\
         - `up` is the anchor's +Y direction.\n\
         - Do NOT output an anchor named \"origin\". The engine provides an implicit identity anchor named \"origin\".\n\n\
          Attachment definition:\n\
          - Each non-root component must define `attach_to`.\n\
          - `attach_to` aligns this component's `child_anchor` to the parent's `parent_anchor`.\n\
          - `offset` is an extra tweak transform expressed in the PARENT ANCHOR JOIN FRAME (after alignment).\n\
          - The engine will NOT apply any hidden auto-placement rules. If you need overlap/inset/outset, encode it explicitly in `offset.pos`.\n\
          - Define attachment anchors as JOIN frames (each expressed in its OWN component-local coordinates):\n\
            - Set `parent_anchor.forward` (+Z) to point from the parent toward the child (attachment direction) in the PARENT component's local axes.\n\
            - Set `child_anchor.forward` (+Z) and `child_anchor.up` (+Y) in the CHILD component's local axes so the child can rotate into the parent's join frame.\n\
              They do NOT need to numerically equal the parent's vectors.\n\
              Example: if a chain link is modeled along the child's local +Z axis, use `forward=[0,0,1]` and `up=[0,1,0]` for its joint anchors.\n\
            - Do NOT make the join frames 180° opposed (that flips the child). If you need a flip, encode it via `attach_to.offset` rotation.\n\
          - Then `offset.pos[2]` becomes a reliable in/out control along the attachment direction.\n\
          - For flush joins, use a small NEGATIVE `offset.pos[2]` (slight inset/overlap). For surface overlays, use a small POSITIVE `offset.pos[2]` (slight outset) so thin details are not buried.\n\
          Motion metadata (optional; recommended for movable objects):\n\
          - Use `attach_to.joint` to declare articulation (`fixed`/`hinge`/`ball`/`free`).\n\
            - Joint axes/limits are expressed in the PARENT ANCHOR JOIN FRAME (the same frame as `attach_to.offset`).\n\
          - Use `contacts[]` to declare ground contacts (feet/hooves) by referencing one of the component's anchors.\n\
            - For planted contacts, include `contacts[].stance`:\n\
              - `phase_01`: start phase in [0,1).\n\
              - `duty_factor_01`: fraction of cycle in (0,1].\n\
          - Optional: top-level `rig.move_cycle_m` defines meters-per-cycle for locomotion.\n\n\
         Reuse groups (IMPORTANT for speed + consistency):\n\
         - If multiple components should share the SAME geometry (wheels, repeated legs, mirrored parts, numbered sets like `leg_0..leg_7`), declare `reuse_groups`.\n\
         - The engine can then generate only the unique components + the declared reuse sources, and fill the remaining targets via deterministic copy.\n\
         - This does NOT change the attachment tree; it only affects how missing geometry gets produced.\n\
         - Reuse targets are allowed (and expected) to differ in per-target `attach_to.offset` (radial/mirrored placement).\n\
         - IMPORTANT: The engine will NOT guess whether a reuse group should be mirrored. You MUST explicitly set `reuse_groups[].alignment`.\n\
         - `reuse_groups[].kind`:\n\
           - `component` (copy a single component's geometry)\n\
           - `subtree` (copy an entire limb-chain subtree rooted at `source`)\n\
         - `reuse_groups[].alignment` (REQUIRED):\n\
           - `rotation` (copy by mount alignment rotation only; use for identical repeated parts)\n\
           - `mirror_mount_x` (mirror across the mount join frame's local +X axis; use for L/R symmetry)\n\
         - `reuse_groups[].mode` (optional; default `detached`): `detached` | `linked`.\n\
           - For `subtree`, prefer `detached`.\n\
         - `reuse_groups[].anchors` (optional; default `preserve_interfaces`):\n\
           - `preserve_interfaces` (recommended; preserves each target's mount interface and any external child-attachment anchors, but copies other anchors so internal anchors stay consistent with the copied geometry)\n\
           - `preserve_target` (legacy; keeps ALL target anchors unchanged; can cause internal-anchor drift when geometry is aligned)\n\
           - `copy_source` (overwrite target anchors to match the source exactly)\n\
         - Field aliases accepted by the engine:\n\
           - `source` may be `source_root` / `source_component`.\n\
           - `targets` may be `target_roots` / `target_components`.\n\n\
          Schema:\n\
          {{\n\
            \"version\": 8,\n\
            \"rig\": {{ \"move_cycle_m\": number }} (optional),\n\
            \"mobility\": {{\"kind\":\"static\"}} | {{\"kind\":\"ground\",\"max_speed\": number}} | {{\"kind\":\"air\",\"max_speed\": number}},\n\
            \"attack\": {{ ... }} (optional; omit if not attack-capable),\n\
            \"aim\": {{\"max_yaw_delta_degrees\": number|null (optional), \"components\": [\"component_name\", ...]}} (optional),\n\
            \"collider\": {{\"kind\":\"aabb_xz\",\"half_extents\":[x,z]}} | {{\"kind\":\"circle_xz\",\"radius\":r}} | {{\"kind\":\"none\"}} (optional),\n\
            \"assembly_notes\": \"...\" (optional),\n\
          \"root_component\": \"component_name\" (optional; otherwise inferred as the only component without attach_to),\n\
          \"reuse_groups\": [\n\
              {{ \"kind\": \"component\" | \"subtree\", \"source\": \"component_name\", \"targets\": [\"component_name\", ...], \"alignment\": \"rotation\" | \"mirror_mount_x\", \"mode\": \"detached\" | \"linked\" (optional), \"anchors\": \"preserve_interfaces\" | \"preserve_target\" | \"copy_source\" (optional) }}\n\
          ] (optional),\n\
          \"components\": [\n\
            {{\n\
              \"name\": \"stable_unique_identifier\",\n\
                \"purpose\": \"...\" (optional),\n\
                \"modeling_notes\": \"...\" (optional),\n\
                \"size\": [x,y,z],\n\
                \"anchors\": [\n\
                  {{\"name\":\"anchor_name\",\"pos\":[x,y,z],\"forward\":[x,y,z],\"up\":[x,y,z]}}\n\
                ],\n\
                \"contacts\": [\n\
                  {{\"name\":\"contact_name\",\"kind\":\"ground\",\"anchor\":\"anchor_name\",\"stance\":{{\"phase_01\": number,\"duty_factor_01\": number}} }}\n\
                ] (optional),\n\
                \"attach_to\": {{\n\
                  \"parent\": \"parent_component_name\",\n\
                  \"parent_anchor\": \"anchor_name_on_parent\",\n\
                  \"child_anchor\": \"anchor_name_on_this_component\",\n\
                      \"offset\": {{\n\
                        \"pos\": [x,y,z],\n\
                        \"forward\": [x,y,z] (optional),\n\
                        \"up\": [x,y,z] (optional),\n\
                        \"rot_frame\": \"join\" | \"parent\",\n\
                        \"rot_quat_xyzw\": [x,y,z,w] (optional),\n\
                        \"scale\": [x,y,z] (optional)\n\
                      }} (optional),\n\
                  \"joint\": {{ \"kind\": \"hinge\" | \"fixed\" | \"ball\" | \"free\", \"axis_join\": [x,y,z] (optional), \"limits_degrees\": [min,max] (optional) }} (optional)\n\
               }} (omit ONLY for the root component)\n\
             }}\n\
           ]\n\
         }}\n\n\
         Constraints:\n\
         - Provide at most {GEN3D_MAX_COMPONENTS} components.\n\
         - Use units roughly in meters; keep scale consistent.\n\
         - Each component should be a coherent structural sub-part (not tiny decoration).\n\
         - Focus on BASIC STRUCTURE over details.\n\
         - Order components from most important (core volumes) to least important.\n\
         - `size` is the component's approximate full bounds extents in the component's LOCAL axes.\n\
         - Keep names simple; no spaces.\n\
         - Output only the JSON object; no markdown.\n"
    )
}

pub(super) fn build_gen3d_component_system_instructions() -> String {
    format!(
        "You are a 3D modeling assistant.\n\
         Return STRICT JSON for a single component.\n\n\
         Requirements:\n\
         - Use ONLY primitives (cuboid, cylinder, sphere, cone).\n\
         - You MUST output `anchors` with the same names as the plan requires.\n\
         - You MUST output per-part `color` as [r,g,b,a] in 0..1 (do not omit).\n\
         - Output only the JSON object; no markdown.\n\n\
         Schema:\n\
         {{\n\
           \"version\": 2,\n\
           \"collider\": {{\"kind\":\"aabb_xz\",\"half_extents\":[x,z]}} | {{\"kind\":\"circle_xz\",\"radius\":r}} | {{\"kind\":\"none\"}} (optional),\n\
           \"anchors\": [\n\
             {{\"name\":\"anchor_name\",\"pos\":[x,y,z],\"forward\":[x,y,z],\"up\":[x,y,z]}}\n\
           ],\n\
           \"parts\": [\n\
             {{\n\
               \"primitive\": \"cuboid\" | \"cylinder\" | \"sphere\" | \"cone\",\n\
               \"params\": {{...}} (optional; only for capsule/conical_frustum/torus etc if supported),\n\
               \"color\": [r,g,b,a],\n\
               \"pos\": [x,y,z],\n\
               \"forward\": [x,y,z] (optional; if present, you MUST also provide `up`),\n\
               \"up\": [x,y,z] (optional; if present, you MUST also provide `forward`),\n\
               \"scale\": [x,y,z]\n\
             }}\n\
           ]\n\
         }}\n\n\
         Constraints:\n\
         - Use units roughly in meters; keep scale consistent with the plan.\n\
         - Component center should be near the origin.\n\
         - Transform semantics: `scale` is a 3D size vector in the part's LOCAL axes (+X right, +Y up, +Z forward), and it rotates with the part when you provide `forward`/`up`.\n\
         - The component's overall local AABB should be close to the plan's `target_size` per-axis (do not swap/permutate axes).\n\
         - Output only the JSON object; no markdown.\n"
    )
}

pub(super) fn build_gen3d_review_delta_system_instructions(review_appearance: bool) -> String {
    let mut out = String::new();
    out.push_str("You are a 3D modeling assistant.\n");
    out.push_str(
        "You will be given structured summaries of the CURRENT assembled draft and smoke checks.\n",
    );
    if review_appearance {
        out.push_str("You will also be given: (1) reference photos (optional) and (2) multiple rendered preview images of the CURRENT assembled draft.\n");
    } else {
        out.push_str(
            "Appearance review is DISABLED by configuration: preview images and reference photos may be omitted.\n",
        );
    }
    out.push_str(
        "Your job is to propose machine-appliable fixes as STRICT JSON.\n\
Return ONLY JSON. Do NOT output markdown.\n\n\
Review mode:\n",
    );
    if review_appearance {
        out.push_str("- appearance_review_enabled: true\n");
    } else {
        out.push_str("- appearance_review_enabled: false\n");
        out.push_str(
            "- Only fix structural issues:\n\
  - smoke_results.motion_validation issues with severity=\"error\"\n\
  - other smoke_results issues with severity=\"error\" (if present)\n\
- Do NOT propose cosmetic-only tweaks, aesthetic regens, or transform nudges.\n\
- Ignore smoke severity=\"warn\" issues unless they imply a severity=\"error\".\n",
        );
    }
    out.push_str(
	        "\nIMPORTANT:\n\
	     - Do NOT assume the engine will \"auto-fix\" placement. If something is wrong, request explicit edits.\n\
	     - Do NOT output Euler angles.\n\
	       - Rotation fields (IMPORTANT):\n\
	         - For `tweak_component_transform.set.rot`, use either:\n\
	           - a basis: {\"forward\":[x,y,z],\"up\":[x,y,z]}\n\
	           - or a quaternion: {\"quat_xyzw\":[x,y,z,w]}\n\
	         - For `tweak_anchor.set`, DO NOT use `rot`. Set `forward` and `up` directly (and optionally `pos`).\n\
	           Example: {\"set\":{\"forward\":[0,0,1],\"up\":[0,1,0]}}\n\
	       - For deltas (`tweak_component_transform.delta` / `tweak_anchor.delta`), use `rot_quat_xyzw` (NOT `quat_xyzw`).\n\
	      - Target components by `component_id` (UUID), not by name.\n\
	      - Keep changes minimal: prefer adjusting attachment offsets / anchors over regenerating geometry.\n\
	      - Focus on HIGH-IMPACT structural issues. If only minor cosmetic tweaks remain, return ONLY {\"kind\":\"accept\"}.\n\
	      - Avoid endless micro-tweaks: if you are satisfied with structure/proportions, accept.\n\
      - If smoke results include `motion_validation.issues`, treat ONLY `severity=error` issues as authoritative and prioritize fixing them first.\n\
",
    );
    if review_appearance {
        out.push_str(
            "        `severity=warn` issues are suggestions; fix them only when it will not regress visuals.\n\n",
        );
    } else {
        out.push_str("        `severity=warn` issues are suggestions; ignore them.\n\n");
    }
    out.push_str(
        "\n\
      Anchor edits and regression safety:\n\
      - `tweak_anchor` is for changing JOINT FRAMES / pivot placement in COMPONENT-LOCAL space.\n\
      - The engine automatically rebases affected attachment offsets when anchors move/rotate so the assembled REST POSE stays stable.\n\
        If you WANT to move/rotate a component in the assembly, use `tweak_component_transform` instead (flat `set`/`delta`; do NOT nest under `offset`).\n\n\
      - If motion validation reports `chain_axis_mismatch`, fix the COMPONENT ANCHORS (not offsets): reorient the child component's joint anchors so the vector from its parent joint anchor to its child joint anchor aligns with the proximal anchor's +Z (forward) in component-local space.\n\n\
      If the scene graph shows no generated geometry yet (e.g. 0 primitive parts / components_generated=0), blank renders are expected.\n\
      Do NOT report that as a renderer bug. Instead, request generating components first.\n\n\
     Attack schema for `tweak_attack` (MUST follow exactly; do not invent custom fields):\n\
     - `attack.kind` must be exactly one of: `none`, `melee`, `ranged_projectile`.\n\
     - Do NOT output synonyms like `cannon`, `gun`, `projectile`, `ranged`; always use the canonical kinds above.\n\
     - If the weapon is a cannon/gun, use `ranged_projectile` and describe it as a cannon in `reason`.\n\
     - `none`: {\"kind\":\"none\"}\n\
     - `melee`: {\"kind\":\"melee\",\"cooldown_secs\":number,\"damage\":integer,\"range\":number,\"radius\":number,\"arc_degrees\":number}\n\
     - `ranged_projectile`: {\"kind\":\"ranged_projectile\",\"cooldown_secs\":number,\"muzzle\":{\"component\":\"<component_name>\",\"anchor\":\"<anchor_name>\"},\"projectile\":{...}}\n\
       - `muzzle.component` MUST be the component NAME (same as in the plan), NOT a UUID.\n\
       - `projectile` MUST include: `shape`, `color` [r,g,b,a], `speed`, `ttl_secs`, `damage` (and shape-specific fields like `radius`/`length`/`size`).\n\n\
     Schema (review_delta_v1):\n\
     {\n\
       \"version\": 1,\n\
       \"applies_to\": {\"run_id\":\"<uuid>\",\"attempt\": N,\"plan_hash\":\"sha256:...\",\"assembly_rev\": N},\n\
       \"summary\": \"...\" (optional),\n\
       \"actions\": [\n\
         {\"kind\":\"accept\"},\n\
         {\"kind\":\"tooling_feedback\",\"feedback\":{...}},\n\
         {\"kind\":\"replan\",\"reason\":\"...\"},\n\
         {\"kind\":\"regen_component\",\"component_id\":\"<uuid>\",\"updated_modeling_notes\":\"...\" (optional),\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_component_transform\",\"component_id\":\"<uuid>\",\"set\":{...} (optional),\"delta\":{...} (optional),\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_anchor\",\"component_id\":\"<uuid>\",\"anchor_name\":\"name\",\"set\":{...} (optional),\"delta\":{...} (optional),\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_attachment\",\"component_id\":\"<uuid>\",\"set\":{...},\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_contact\",\"component_id\":\"<uuid>\",\"contact_name\":\"name\",\"stance\": null (optional),\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_mobility\",\"mobility\":{...},\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_attack\",\"attack\":{...},\"reason\":\"...\" (optional)}\n\
       ]\n\
     }\n\n\
     Notes:\n\
     - `tweak_component_transform` edits the component's attachment OFFSET relative to its parent.\n\
     - `tweak_contact` edits the component's declared contacts (most commonly `stance`).\n\
       - Set stance: `\"stance\": {\"phase_01\": number, \"duty_factor_01\": number}`\n\
       - Clear stance: `\"stance\": null`\n\
       If a stance schedule is wrong, fix it; do NOT clear stance just to silence errors.\n\
     - IMPORTANT: For `tweak_component_transform`, `set.pos` / `delta.pos` are expressed in the PARENT ANCHOR join frame (same frame as `attach_to.offset.pos`).\n\
       In that join frame, `pos = [x,y,z]` means:\n\
       - +X (`pos[0]`) is `join_right_world`\n\
       - +Y (`pos[1]`) is `join_up_world`\n\
       - +Z (`pos[2]`) is `join_forward_world` (this is `parent_anchor.forward`, defined to point from the parent toward the child)\n\
       Therefore:\n\
       - If a child looks DETACHED and you want to pull it TOWARD its parent, use a NEGATIVE `delta.pos[2]` (decrease `pos[2]`).\n\
       - If you want to push a child further away along the attachment direction, use a POSITIVE `delta.pos[2]` (increase `pos[2]`).\n\
       - If you want to apply a desired WORLD-space translation delta `W = [wx,wy,wz]`, convert it into join-frame deltas:\n\
         - `delta.pos[0] = dot(W, join_right_world)`\n\
         - `delta.pos[1] = dot(W, join_up_world)`\n\
         - `delta.pos[2] = dot(W, join_forward_world)`\n\
     - `tooling_feedback` is for missing tools / tool improvements / tool bugs.\n\
       It must include at least: {\"version\":1,\"priority\":\"low|medium|high|blocker\",\"title\":\"...\",\"summary\":\"...\"}.\n\
       You may include additional fields (e.g. `missing_tools`, `enhancements`, `bugs`, `examples`, `details`).\n\
     - Use `replan` only if the component breakdown or attachment tree is fundamentally wrong.\n\
     - Use `regen_component` if the component geometry is structurally wrong (not for tiny details).\n"
    );
    out
}

pub(super) fn build_gen3d_review_delta_user_text(
    run_id: &str,
    attempt: u32,
    plan_hash: &str,
    assembly_rev: u32,
    raw_prompt: &str,
    has_images: bool,
    scene_graph_summary: &serde_json::Value,
    smoke_results: &serde_json::Value,
) -> String {
    fn fmt_f32(v: Option<f32>) -> String {
        v.map(|f| format!("{f:.2}"))
            .unwrap_or_else(|| "null".into())
    }

    fn read_vec3(value: &serde_json::Value) -> Option<(f32, f32, f32)> {
        let arr = value.as_array()?;
        if arr.len() != 3 {
            return None;
        }
        Some((
            arr[0].as_f64()? as f32,
            arr[1].as_f64()? as f32,
            arr[2].as_f64()? as f32,
        ))
    }

    fn read_vec2(value: &serde_json::Value) -> Option<(f32, f32)> {
        let arr = value.as_array()?;
        if arr.len() != 2 {
            return None;
        }
        Some((arr[0].as_f64()? as f32, arr[1].as_f64()? as f32))
    }

    fn fmt_vec3(value: Option<(f32, f32, f32)>) -> String {
        match value {
            Some((x, y, z)) => format!("[{:.2},{:.2},{:.2}]", x, y, z),
            None => "null".into(),
        }
    }

    fn fmt_vec2(value: Option<(f32, f32)>) -> String {
        match value {
            Some((x, y)) => format!("[{:.2},{:.2}]", x, y),
            None => "null".into(),
        }
    }

    fn compact_scene_graph_summary(summary: &serde_json::Value) -> String {
        fn normalized_cross(a: (f32, f32, f32), b: (f32, f32, f32)) -> Option<(f32, f32, f32)> {
            let (ax, ay, az) = a;
            let (bx, by, bz) = b;
            let rx = ay * bz - az * by;
            let ry = az * bx - ax * bz;
            let rz = ax * by - ay * bx;
            let len2 = rx * rx + ry * ry + rz * rz;
            if !len2.is_finite() || len2 <= 1e-6 {
                return None;
            }
            let inv = 1.0 / len2.sqrt();
            Some((rx * inv, ry * inv, rz * inv))
        }

        let mut out = String::new();

        if let Some(root) = summary.get("root") {
            let size = root.get("size").and_then(read_vec3);
            let mobility = root
                .get("mobility")
                .and_then(|m| m.get("kind"))
                .and_then(|v| v.as_str());
            let speed = root
                .get("mobility")
                .and_then(|m| m.get("max_speed"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32);
            let attack_kind = root
                .get("attack")
                .and_then(|a| a.get("kind"))
                .and_then(|v| v.as_str());
            let collider_kind = root
                .get("collider")
                .and_then(|c| c.get("kind"))
                .and_then(|v| v.as_str());
            let half = root
                .get("collider")
                .and_then(|c| c.get("half_extents"))
                .and_then(read_vec2);

            out.push_str(&format!(
                "- root: size={} mobility={} max_speed={} attack={} collider={} half_extents={}\n",
                fmt_vec3(size),
                mobility.unwrap_or("unknown"),
                fmt_f32(speed),
                attack_kind.unwrap_or("none"),
                collider_kind.unwrap_or("none"),
                fmt_vec2(half),
            ));
        }

        let Some(components) = summary.get("components").and_then(|v| v.as_array()) else {
            return out;
        };

        out.push_str(&format!("- components_total: {}\n", components.len()));
        for c in components {
            let name = c
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("<unnamed>");
            let id = c
                .get("component_id_uuid")
                .and_then(|v| v.as_str())
                .unwrap_or("<no-id>");
            let parent_name = c
                .get("attach_to")
                .and_then(|a| a.get("parent_component_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("(root)");
            let parent_id = c
                .get("attach_to")
                .and_then(|a| a.get("parent_component_id_uuid"))
                .and_then(|v| v.as_str());
            let parent_anchor = c
                .get("attach_to")
                .and_then(|a| a.get("parent_anchor"))
                .and_then(|v| v.as_str());
            let child_anchor = c
                .get("attach_to")
                .and_then(|a| a.get("child_anchor"))
                .and_then(|v| v.as_str());

            let pos = c
                .get("resolved_transform")
                .and_then(|t| t.get("pos"))
                .and_then(read_vec3);
            let forward = c
                .get("resolved_transform")
                .and_then(|t| t.get("forward"))
                .and_then(read_vec3);
            let up = c
                .get("resolved_transform")
                .and_then(|t| t.get("up"))
                .and_then(read_vec3);

            let planned_size = c.get("planned_size").and_then(read_vec3);
            let actual_size = c.get("actual_size").and_then(read_vec3);

            let offset_pos_join = c
                .get("attach_to")
                .and_then(|a| a.get("offset"))
                .and_then(|o| o.get("pos"))
                .and_then(read_vec3);
            let joint_kind = c
                .get("attach_to")
                .and_then(|a| a.get("joint"))
                .and_then(|j| j.get("kind"))
                .and_then(|v| v.as_str());
            let joint_axis_join = c
                .get("attach_to")
                .and_then(|a| a.get("joint"))
                .and_then(|j| j.get("axis_join"))
                .and_then(read_vec3);
            let joint_limits_degrees = c
                .get("attach_to")
                .and_then(|a| a.get("joint"))
                .and_then(|j| j.get("limits_degrees"))
                .and_then(read_vec2);
            let join_forward_world = c
                .get("attach_to")
                .and_then(|a| a.get("join_forward_world"))
                .and_then(read_vec3);
            let join_up_world = c
                .get("attach_to")
                .and_then(|a| a.get("join_up_world"))
                .and_then(read_vec3);
            let join_right_world = c
                .get("attach_to")
                .and_then(|a| a.get("join_right_world"))
                .and_then(read_vec3)
                .or_else(|| {
                    join_up_world
                        .zip(join_forward_world)
                        .and_then(|(u, f)| normalized_cross(u, f))
                });

            let anchors: Vec<&str> = c
                .get("anchors")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
                        .collect()
                })
                .unwrap_or_default();

            out.push_str(&format!("- {name}: id={id} parent={parent_name}"));
            if let Some(parent_id) = parent_id {
                out.push_str(&format!(" parent_id={parent_id}"));
            }
            if let Some(parent_anchor) = parent_anchor {
                out.push_str(&format!(" parent_anchor={parent_anchor}"));
            }
            if let Some(child_anchor) = child_anchor {
                out.push_str(&format!(" child_anchor={child_anchor}"));
            }
            out.push_str(&format!(
                " pos={} fwd={} up={} planned_size={} actual_size={}",
                fmt_vec3(pos),
                fmt_vec3(forward),
                fmt_vec3(up),
                fmt_vec3(planned_size),
                fmt_vec3(actual_size)
            ));

            if parent_name != "(root)" {
                if let Some(joint_kind) = joint_kind {
                    out.push_str(&format!(" joint={joint_kind}"));
                    if joint_kind == "hinge" {
                        if joint_axis_join.is_some() {
                            out.push_str(&format!(" axis_join={}", fmt_vec3(joint_axis_join)));
                        }
                        if joint_limits_degrees.is_some() {
                            out.push_str(&format!(
                                " limits_deg={}",
                                fmt_vec2(joint_limits_degrees)
                            ));
                        }
                    }
                }
                out.push_str(&format!(
                    " offset.pos(join_frame)={} join_right_world={} join_up_world={} join_forward_world={}",
                    fmt_vec3(offset_pos_join),
                    fmt_vec3(join_right_world),
                    fmt_vec3(join_up_world),
                    fmt_vec3(join_forward_world),
                ));
            }

            out.push_str(&format!(" anchors={:?}\n", anchors));
        }

        out
    }

    let mut out = String::new();
    out.push_str("Auto-review pass: propose strict machine-appliable deltas.\n");
    if !has_images {
        out.push_str("No photos are provided for this run.\n");
    }
    out.push_str(&build_gen3d_effective_user_prompt(raw_prompt));
    out.push('\n');

    out.push_str("You MUST copy these values into `applies_to` exactly:\n");
    out.push_str(&format!("- run_id: {run_id}\n"));
    out.push_str(&format!("- attempt: {attempt}\n"));
    out.push_str(&format!("- plan_hash: {plan_hash}\n"));
    out.push_str(&format!("- assembly_rev: {assembly_rev}\n\n"));

    out.push_str("scene_graph_summary (compact):\n");
    out.push_str(&compact_scene_graph_summary(scene_graph_summary));
    out.push_str("\n\nsmoke_results.json:\n");
    out.push_str(
        &serde_json::to_string_pretty(smoke_results).unwrap_or_else(|_| smoke_results.to_string()),
    );
    out.push('\n');
    out
}

pub(super) fn build_gen3d_descriptor_meta_system_instructions() -> String {
    "You are a metadata assistant for Gravimera.\n\
You will be given information about a generated 3D prefab (prompt + derived facts + AI plan extract).\n\
Return ONLY a single JSON object for gen3d_descriptor_meta_v1 (no markdown, no prose).\n\n\
Schema:\n\
{\n\
  \"version\": 1,\n\
  \"short\": \"1–2 lines: an AI conclusion describing what the prefab is (do NOT copy the user prompt)\",\n\
  \"tags\": [\"lower_snake_case\", \"searchable\", \"style\", \"species\", \"category\", \"materials\", \"theme\"]\n\
}\n\n\
Rules:\n\
- `short` must be a conclusion based on the provided facts (not the prompt text).\n\
- Prefer concrete nouns/adjectives; avoid marketing fluff.\n\
- `tags` must be lower_snake_case tokens (no spaces, no hyphens).\n\
- Include tags that help scene building search (style/species/category/theme/material/function).\n\
- Keep tags stable and generic (open vocabulary); avoid internal ids/UUIDs.\n"
        .to_string()
}

pub(super) fn build_gen3d_motion_roles_system_instructions() -> String {
    "You are the Gravimera Gen3D motion roles mapper.\n\
You will be given a generated component graph (names + attachments + transforms) and must label motion effectors for generic runtime animation.\n\
Return ONLY a single JSON object for gen3d_motion_roles_v1 (no markdown, no prose).\n\n\
Schema:\n\
{\n\
  \"version\": 1,\n\
  \"applies_to\": {\"run_id\":\"uuid\",\"attempt\":0,\"plan_hash\":\"sha256:...\",\"assembly_rev\":0},\n\
  \"move_effectors\": [\n\
    {\"component\":\"left_thigh\",\"role\":\"leg\",\"phase_group\":0,\"spin_axis_local\":null},\n\
    {\"component\":\"wheel_fl\",\"role\":\"wheel\",\"phase_group\":null,\"spin_axis_local\":[1,0,0]},\n\
    {\"component\":\"left_arm\",\"role\":\"arm\",\"phase_group\":0,\"spin_axis_local\":null},\n\
    {\"component\":\"head\",\"role\":\"head\",\"phase_group\":null,\"spin_axis_local\":null},\n\
    {\"component\":\"ear_l\",\"role\":\"ear\",\"phase_group\":null,\"spin_axis_local\":null},\n\
    {\"component\":\"propeller\",\"role\":\"propeller\",\"phase_group\":null,\"spin_axis_local\":[0,0,1]}\n\
  ],\n\
  \"notes\": \"...\" | null\n\
}\n\n\
Rules:\n\
- You MUST copy the provided applies_to values exactly.\n\
- `move_effectors` targets attachment edges by naming the CHILD component (`component`).\n\
  - Only output component names that appear in the provided component list.\n\
  - Do NOT output the root component (it has no parent edge).\n\
- Allowed `role` values:\n\
  - `leg`, `wheel`, `arm`, `head`, `ear`, `tail`, `wing`, `propeller`, `rotor`.\n\
- When the prompt implies a tool-like melee action (e.g. digging), include the articulated tool-arm joints as `role=\"arm\"`.\n\
- `phase_group`:\n\
  - For `leg`, set to 0 or 1 (two-phase gait: group 0 swings opposite group 1).\n\
  - For `arm`, set to 0 or 1 when it is part of a walking gait; otherwise it may be null.\n\
  - For `wheel`/`propeller`/`rotor`, MUST be null.\n\
- `spin_axis_local`:\n\
  - For `wheel`/`propeller`/`rotor`, may be null or a unit-ish axis like [1,0,0].\n\
  - For other roles, MUST be null.\n\
- If you cannot confidently identify any effectors, return an EMPTY `move_effectors` list.\n"
        .to_string()
}

pub(super) fn build_gen3d_motion_roles_user_text(
    raw_prompt: &str,
    has_images: bool,
    run_id: &str,
    attempt: u32,
    plan_hash: &str,
    assembly_rev: u32,
    components: &[Gen3dPlannedComponent],
) -> String {
    fn fmt_vec3(v: Vec3) -> String {
        format!("[{:.3},{:.3},{:.3}]", v.x, v.y, v.z)
    }

    let mut out = String::new();
    out.push_str(
        "Goal: label motion roles so the engine can inject generic move + attack animations.\n",
    );
    if !has_images {
        out.push_str("No photos are provided for this run.\n");
    }
    out.push_str(&build_gen3d_effective_user_prompt(raw_prompt));
    out.push('\n');

    out.push_str("You MUST copy these values into `applies_to` exactly:\n");
    out.push_str(&format!("- run_id: {run_id}\n"));
    out.push_str(&format!("- attempt: {attempt}\n"));
    out.push_str(&format!("- plan_hash: {plan_hash}\n"));
    out.push_str(&format!("- assembly_rev: {assembly_rev}\n\n"));

    out.push_str("Components (resolved transforms in root frame):\n");
    for c in components {
        let forward = c.rot * Vec3::Z;
        let up = c.rot * Vec3::Y;
        let parent = c
            .attach_to
            .as_ref()
            .map(|att| att.parent.as_str())
            .unwrap_or("<root>");
        let size = c.actual_size.unwrap_or(c.planned_size);
        out.push_str(&format!(
            "- name={} parent={} pos={} forward={} up={} size={} contacts={}\n",
            c.name.trim(),
            parent.trim(),
            fmt_vec3(c.pos),
            fmt_vec3(forward),
            fmt_vec3(up),
            fmt_vec3(size),
            c.contacts.len()
        ));
    }

    out
}

pub(super) fn build_gen3d_descriptor_meta_user_text(
    prefab_label: &str,
    user_prompt: &str,
    roles: &[String],
    size_m: Vec3,
    ground_origin_y_m: f32,
    mobility: Option<&str>,
    attack_kind: Option<&str>,
    anchors: &[String],
    animation_channels: &[String],
    plan_extracted_text: Option<&str>,
    motion_summary_json: Option<&serde_json::Value>,
) -> String {
    let mut out = String::new();
    out.push_str("Generate searchable semantic metadata for this prefab.\n\n");

    let label = prefab_label.trim();
    if !label.is_empty() {
        out.push_str(&format!("Prefab label: {label}\n"));
    } else {
        out.push_str("Prefab label: <none>\n");
    }

    out.push_str(&format!(
        "Size (m): [{:.3}, {:.3}, {:.3}]\n",
        size_m.x, size_m.y, size_m.z
    ));
    out.push_str(&format!("ground_origin_y (m): {ground_origin_y_m:.3}\n"));
    out.push_str(&format!("Mobility: {}\n", mobility.unwrap_or("static")));
    out.push_str(&format!("Attack: {}\n", attack_kind.unwrap_or("none")));
    out.push_str(&format!("Roles: {:?}\n", roles));
    out.push_str(&format!("Anchors: {:?}\n", anchors));
    out.push_str(&format!("Animation channels: {:?}\n", animation_channels));

    if let Some(summary) = motion_summary_json {
        out.push_str("\nMotion summary (derived JSON):\n");
        out.push_str(
            &serde_json::to_string_pretty(summary).unwrap_or_else(|_| summary.to_string()),
        );
        out.push('\n');
    }

    let prompt = user_prompt.trim();
    if prompt.is_empty() {
        out.push_str("\nUser prompt: (none)\n");
    } else {
        out.push_str("\nUser prompt (context only; DO NOT copy as short):\n");
        out.push_str(prompt);
        out.push('\n');
    }

    if let Some(plan) = plan_extracted_text
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        out.push_str("\nAI plan extract (more context):\n");
        out.push_str(plan);
        out.push('\n');
    }

    out
}
