use bevy::prelude::*;

use super::schema::{AiContactKindJson, AiJointKindJson};
use super::Gen3dPlannedComponent;
use crate::gen3d::state::{Gen3dDraft, Gen3dSpeedMode};

use crate::gen3d::{
    GEN3D_DEFAULT_STYLE_PROMPT, GEN3D_IMAGE_OBJECT_SUMMARY_MAX_WORDS, GEN3D_MAX_COMPONENTS,
    GEN3D_MAX_PARTS,
};

pub(super) fn build_gen3d_user_image_object_summary_system_instructions() -> String {
    format!(
        "You are an image-to-text summarizer for Gravimera Gen3D.\n\
You will be given 1–3 reference photos of an object.\n\
Your job: describe ONLY the MAIN object (ignore the background) so a text-only 3D modeling agent can generate it.\n\n\
This summary is used to rebuild a 3D model using basic 3D primitives, so prioritize:\n\
- overall silhouette and MAJOR VOLUMES,\n\
- part-to-part proportions/ratios,\n\
- a primitives-first “blockout” description.\n\n\
Output format:\n\
- Plain text only.\n\
- Use 8–10 bullets.\n\
- Every bullet MUST start with `- `.\n\
- Keep the ENTIRE output to at most {GEN3D_IMAGE_OBJECT_SUMMARY_MAX_WORDS} whitespace-separated words.\n\
- Be concise: aim for ~160–200 words unless the object is unusually complex.\n\n\
Bullet labels (use these; one per bullet):\n\
- Primary object:\n\
- Geometry/silhouette:\n\
- Proportions:\n\
- Parts/topology:\n\
- Primitive blockout:\n\
- Symmetry/repetition:\n\
- Materials/surface:\n\
- Colors:\n\
- Style:\n\
- Notable details:\n\
- View/occlusions:\n\
- Unknowns:\n\
- Certainty: high|medium|low\n\n\
Rules:\n\
- Only describe clearly visible facts.\n\
- For Proportions: include approximate RELATIVE ratios (examples: head≈1/6 of height; torso wider than hips; limbs thin/thick vs torso). If it’s not a body/character, use part-to-part ratios (example: handle length vs blade length).\n\
- For Primitive blockout: describe how to approximate the visible shape using basic primitives (cuboid/sphere/cylinder/cone) and how they relate in position/scale.\n\
- If unsure, put it under Unknowns (do NOT guess).\n\
- Do not describe background, lighting, or camera unless it changes object identity.\n"
    )
}

pub(super) fn build_gen3d_user_image_object_summary_user_text(
    raw_prompt: &str,
    images_count: usize,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Summarize the main object across these {images_count} reference image(s).\n"
    ));
    let notes = raw_prompt.trim();
    if notes.is_empty() {
        out.push_str("User notes: (none)\n");
    } else {
        out.push_str(
            "User notes (may be incomplete or wrong; if they conflict with the images, mention the mismatch under Unknowns):\n",
        );
        out.push_str(notes);
        out.push('\n');
    }
    out
}

pub(super) fn build_gen3d_prompt_intent_system_instructions() -> String {
    "You are a prompt-intent classifier for Gravimera Gen3D.\n\
Your job: decide whether the requested object MUST have gameplay attack capability (a root attack profile).\n\n\
Definitions:\n\
- requires_attack=true means the user wants the object to be able to perform an attack action (ex: bite/claw, punch, swing a weapon, shoot/projectiles/lasers, cast an offensive spell, explode to damage others).\n\
- requires_attack=false means the user does NOT request attack capability (ex: decorative statue, harmless animal, prop) OR explicitly says it cannot/should not attack.\n\n\
Rules:\n\
- Be language-agnostic: the user notes can be any language.\n\
- If the prompt is ambiguous, set requires_attack=false.\n\
- Output MUST be a single JSON object that matches the schema exactly (no extra text)."
        .to_string()
}

pub(super) fn build_gen3d_prompt_intent_user_text(
    raw_prompt: &str,
    image_object_summary: Option<&str>,
) -> String {
    let mut out = String::new();
    let notes = raw_prompt.trim();
    if notes.is_empty() {
        out.push_str("User notes: (none)\n");
    } else {
        out.push_str("User notes:\n");
        out.push_str(notes);
        out.push('\n');
    }

    if let Some(summary) = image_object_summary
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        out.push_str("Reference photo main-object summary (engine-generated text):\n");
        out.push_str(summary);
        out.push('\n');
    } else {
        out.push_str("Reference photo main-object summary: (none)\n");
    }

    out.push_str(
        "Question: Does the user request that the object can attack in gameplay?\n\
Return JSON with {\"version\":1,\"requires_attack\":true|false}.\n",
    );
    out
}

pub(super) fn build_gen3d_effective_user_prompt(
    raw_prompt: &str,
    image_object_summary: Option<&str>,
) -> String {
    let trimmed = raw_prompt.trim();
    let mut out = String::new();
    out.push_str(
        "Reference inputs:\n\
- If reference photos were provided, the engine pre-processed them into a short text summary.\n\
- Some calls may also include a small number of the raw reference images.\n\
  - If images are included in this request, use them as primary ground truth.\n\
  - Otherwise, rely on the text summary only.\n\
- Only use the user notes and/or the photo summary/images; do NOT invent missing details.\n",
    );
    if let Some(summary) = image_object_summary
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        out.push_str("Reference photo main-object summary (auto-generated; visible facts only):\n");
        out.push_str(summary);
        out.push('\n');
    } else {
        out.push_str("Reference photo main-object summary: (none)\n");
    }
    out.push_str(
        "Conflict rule: if user notes conflict with the photo summary, prefer the user notes.\n",
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
    image_object_summary: Option<&str>,
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
         For movable units (mobility ground/air), you MUST also choose a top-level `collider` sized to the MAIN BODY footprint only (selection/click hit area).\n\
         Use `attach_to.offset.pos` to explicitly encode overlap/inset/outset at joins (the engine will not auto-adjust placement).\n\
         Avoid z-fighting at joins: do NOT make parent/child faces flush and coplanar; add a small epsilon offset along the attachment direction (e.g. `attach_to.offset.pos[2]` ~= 0.005m).\n\
         Define attachment anchors as JOIN frames (each expressed in its OWN component-local coordinates):\n\
            - Set `parent_anchor.forward` (+Z) to point from the parent toward the child (attachment direction) in the PARENT component's local axes.\n\
          - Set `child_anchor.forward` (+Z) and `child_anchor.up` (+Y) in the CHILD component's local axes so the child can rotate into the parent's join frame.\n\
            They do NOT need to numerically equal the parent's vectors.\n\
            Example: if a chain link is modeled along the child's local +Z axis, use `forward=[0,0,1]` and `up=[0,1,0]` for its joint anchors.\n\
          - Engine constraint (strict): for EVERY attachment, the join anchor axes must be in the same hemisphere.\n\
            - Require: dot(parent_anchor.forward, child_anchor.forward) > 0 AND dot(parent_anchor.up, child_anchor.up) > 0.\n\
              (This dot check is applied as an engine guardrail in component-local coordinates.)\n\
            - If either dot is negative, FIX the anchor bases (flip the child anchor's forward/up as needed) and/or encode the flip via `attach_to.offset` rotation.\n\
              Do NOT rely on opposed anchors.\n\
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
    out.push_str(&build_gen3d_effective_user_prompt(
        raw_prompt,
        image_object_summary,
    ));
    out
}

pub(super) fn build_gen3d_plan_user_text_with_hints(
    raw_prompt: &str,
    image_object_summary: Option<&str>,
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
         For movable units (mobility ground/air), you MUST also choose a top-level `collider` sized to the MAIN BODY footprint only (selection/click hit area).\n\
         Use `attach_to.offset.pos` to explicitly encode overlap/inset/outset at joins (the engine will not auto-adjust placement).\n\
         Avoid z-fighting at joins: do NOT make parent/child faces flush and coplanar; add a small epsilon offset along the attachment direction (e.g. `attach_to.offset.pos[2]` ~= 0.005m).\n\
         Define attachment anchors as JOIN frames (each expressed in its OWN component-local coordinates):\n\
            - Set `parent_anchor.forward` (+Z) to point from the parent toward the child (attachment direction) in the PARENT component's local axes.\n\
          - Set `child_anchor.forward` (+Z) and `child_anchor.up` (+Y) in the CHILD component's local axes so the child can rotate into the parent's join frame.\n\
            They do NOT need to numerically equal the parent's vectors.\n\
            Example: if a chain link is modeled along the child's local +Z axis, use `forward=[0,0,1]` and `up=[0,1,0]` for its joint anchors.\n\
          - Engine constraint (strict): for EVERY attachment, the join anchor axes must be in the same hemisphere.\n\
            - Require: dot(parent_anchor.forward, child_anchor.forward) > 0 AND dot(parent_anchor.up, child_anchor.up) > 0.\n\
              (This dot check is applied as an engine guardrail in component-local coordinates.)\n\
            - If either dot is negative, FIX the anchor bases (flip the child anchor's forward/up as needed) and/or encode the flip via `attach_to.offset` rotation.\n\
              Do NOT rely on opposed anchors.\n\
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

    out.push_str(&build_gen3d_effective_user_prompt(
        raw_prompt,
        image_object_summary,
    ));
    out
}

pub(super) fn build_gen3d_plan_user_text_preserve_existing_components(
    raw_prompt: &str,
    image_object_summary: Option<&str>,
    speed: Gen3dSpeedMode,
    style_hint: Option<&str>,
    existing_components: &[Gen3dPlannedComponent],
    existing_assembly_notes: &str,
    preserve_edit_policy: &str,
    rewire_components: &[String],
    plan_template: Option<&serde_json::Value>,
) -> String {
    let mut out = String::new();
    out.push_str(
        "EDIT MODE (preserve existing components): You are planning changes to an ALREADY-GENERATED Gen3D draft.\n\
Hard requirements:\n\
- Keep ALL existing component names; do NOT rename or delete existing components.\n\
- Preserve existing geometry: avoid requiring regeneration of existing components unless strictly necessary.\n\
- Prefer \"no-regeneration\" strategies when possible:\n\
  - If the request can be fulfilled by adding/modifying primitive parts on an existing component via `apply_draft_ops_v1`, do NOT add new components.\n\
  - If a new component is necessary, attach it to existing components using existing anchors (or implicit `origin`) and explicit `attach_to.offset`.\n\
- Keep mobility/attack/aim/collider unchanged unless the user request requires changing behavior.\n\
",
    );

    out.push_str("\nPreserve-mode edit policy:\n- preserve_edit_policy: ");
    out.push_str(preserve_edit_policy.trim());
    out.push('\n');
    if preserve_edit_policy.trim() == "allow_rewire" {
        out.push_str("- rewire_components (explicit allow-list): ");
        if rewire_components.is_empty() {
            out.push_str("(none)\n");
        } else {
            for (idx, name) in rewire_components.iter().take(24).enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(name.trim());
            }
            if rewire_components.len() > 24 {
                out.push_str(", …");
            }
            out.push('\n');
        }
    }

    if !existing_assembly_notes.trim().is_empty() {
        out.push_str("\nExisting assembly_notes (context; keep consistent unless needed):\n");
        out.push_str(existing_assembly_notes.trim());
        out.push('\n');
    }

    out.push_str("\nExisting component snapshot (names + interfaces):\n");
    out.push_str("Note: some seeded components can have `actual_size` != `planned_size`. When adding new anchors/components, use `actual_size` for scale.\n");
    if existing_components.is_empty() {
        out.push_str("- (none)\n");
    } else {
        for comp in existing_components
            .iter()
            .take(GEN3D_MAX_COMPONENTS.min(64))
        {
            let is_root = comp.attach_to.is_none();
            out.push_str("- ");
            out.push_str(comp.name.as_str());
            if is_root {
                out.push_str(" (root)");
            } else if let Some(att) = comp.attach_to.as_ref() {
                let t = att.offset.translation;
                let r = att.offset.rotation;
                let s = att.offset.scale;
                out.push_str(&format!(
                    " attach_to parent={} parent_anchor={} child_anchor={} offset.pos=[{:.6},{:.6},{:.6}] offset.rot_quat_xyzw=[{:.6},{:.6},{:.6},{:.6}] offset.scale=[{:.6},{:.6},{:.6}]",
                    att.parent,
                    att.parent_anchor,
                    att.child_anchor,
                    t.x,
                    t.y,
                    t.z,
                    r.x,
                    r.y,
                    r.z,
                    r.w,
                    s.x,
                    s.y,
                    s.z
                ));
                if let Some(joint) = att.joint.as_ref() {
                    let kind = match joint.kind {
                        AiJointKindJson::Fixed => "fixed",
                        AiJointKindJson::Hinge => "hinge",
                        AiJointKindJson::Ball => "ball",
                        AiJointKindJson::Free => "free",
                        AiJointKindJson::Unknown => "unknown",
                    };
                    out.push_str(&format!(" joint.kind={kind}"));
                    if let Some(axis) = joint.axis_join {
                        out.push_str(&format!(
                            " joint.axis_join=[{:.3},{:.3},{:.3}]",
                            axis[0], axis[1], axis[2]
                        ));
                    }
                    if let Some([min_deg, max_deg]) = joint.limits_degrees {
                        out.push_str(&format!(
                            " joint.limits_degrees=[{min_deg:.1},{max_deg:.1}]"
                        ));
                    }
                    if let Some([min_deg, max_deg]) = joint.swing_limits_degrees {
                        out.push_str(&format!(
                            " joint.swing_limits_degrees=[{min_deg:.1},{max_deg:.1}]"
                        ));
                    }
                    if let Some([min_deg, max_deg]) = joint.twist_limits_degrees {
                        out.push_str(&format!(
                            " joint.twist_limits_degrees=[{min_deg:.1},{max_deg:.1}]"
                        ));
                    }
                }
            }
            out.push_str(&format!(
                " planned_size=[{:.3},{:.3},{:.3}]",
                comp.planned_size.x, comp.planned_size.y, comp.planned_size.z
            ));
            if let Some(actual) = comp.actual_size {
                out.push_str(&format!(
                    " actual_size=[{:.3},{:.3},{:.3}]",
                    actual.x, actual.y, actual.z
                ));
            }
            out.push('\n');

            // List anchor names only; keep this compact to preserve context budget.
            if !comp.anchors.is_empty() {
                out.push_str("  anchors: [");
                let mut first = true;
                for a in comp.anchors.iter().take(24) {
                    let name = a.name.as_ref().trim();
                    if name.is_empty() {
                        continue;
                    }
                    if !first {
                        out.push_str(", ");
                    }
                    first = false;
                    out.push_str(name);
                }
                if comp.anchors.len() > 24 {
                    out.push_str(", …");
                }
                out.push_str("]\n");
            }
        }
    }

    if let Some(template) = plan_template {
        out.push_str("\nPlan template (engine-generated; copy+edit):\n");
        out.push_str(&serde_json::to_string(template).unwrap_or_else(|_| template.to_string()));
        out.push('\n');
        out.push_str(
            "Template usage:\n\
- Start from the template and make the smallest valid changes.\n\
- Keep ALL existing component names and keep the same root component.\n\
- Preserve existing anchor frames unless adding NEW anchors.\n\
- Only change existing attach_to fields if the preserve_edit_policy allows it.\n",
        );
    }

    out.push_str("\n---\n\n");

    out.push_str("Planning (preserve-mode patch):\n");
    out.push_str(
        "You are PATCHING an existing plan, not designing from scratch.\n\
- Output a complete plan JSON that includes ALL existing component names.\n\
- Unless the policy explicitly allows it, keep every existing component's `attach_to` (parent, anchors, offset) unchanged.\n\
- Prefer adding new components and/or adding new anchors to existing components.\n\
- Keep mobility/attack/aim/collider unchanged unless the user request requires changing behavior.\n\
",
    );
    match preserve_edit_policy.trim() {
        "additive" => {
            out.push_str(
                "Policy details (additive):\n\
- Do NOT rewire existing components (do not change attach_to.parent / parent_anchor / child_anchor).\n\
- Do NOT move existing components (do not change attach_to.offset).\n\
- Only add new components and/or add new anchors.\n\
- For repositioning, use `apply_draft_ops_v1` (SetAttachmentOffset / SetAnchorTransform) instead of replanning.\n",
            );
        }
        "allow_offsets" => {
            out.push_str(
                "Policy details (allow_offsets):\n\
- Do NOT rewire existing components (do not change attach_to.parent / parent_anchor / child_anchor).\n\
- You MAY change attach_to.offset for existing components to adjust placement.\n\
- Only add new components and/or add new anchors.\n",
            );
        }
        "allow_rewire" => {
            out.push_str(
                "Policy details (allow_rewire):\n\
- You MAY rewire ONLY the components named in `rewire_components` (explicit allow-list).\n\
- For all other existing components, do NOT rewire and do NOT change offsets.\n\
",
            );
        }
        _ => {}
    }

    out.push_str(
        "Join frame reminder:\n\
- For EVERY attachment, the join anchor axes must be in the same hemisphere:\n\
  dot(parent_anchor.forward, child_anchor.forward) > 0 AND dot(parent_anchor.up, child_anchor.up) > 0.\n\
- If you need a flip, encode it via `attach_to.offset` rotation; do NOT rely on opposed anchors.\n\
- If you author any `attach_to.offset` rotation (`forward`/`up` or `rot_quat_xyzw`), you MUST set `attach_to.offset.rot_frame` explicitly (`join` or `parent`) or the engine will reject the plan.\n",
    );

    out.push_str(&format!("Speed mode: {}.\n", speed.label()));
    out.push_str(&format!(
        "Hard cap: at most {} components.\n",
        super::max_components_for_speed(speed)
    ));

    if let Some(style) = style_hint.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        out.push_str("Additional style preference (use this unless the user notes forbid it): ");
        out.push_str(style);
        out.push('\n');
    }
    out.push_str(&build_gen3d_effective_user_prompt(
        raw_prompt,
        image_object_summary,
    ));
    out
}

pub(super) fn build_gen3d_component_user_text(
    raw_prompt: &str,
    image_object_summary: Option<&str>,
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
         Primitive axis reminder (IMPORTANT):\n\
         - For `cylinder` and `cone`, the shape's length/height axis is the part's local +Y axis.\n\
           - Use `scale.y` to control length/height.\n\
           - Use `scale.x` and `scale.z` to control thickness (keep x≈z for round cross-sections).\n\
           - To aim a cylinder/cone along a direction D, set part `up = D` and choose any perpendicular `forward`.\n\
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
    out.push_str(&build_gen3d_effective_user_prompt(
        raw_prompt,
        image_object_summary,
    ));
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
                if let crate::object::registry::PartAnimationDef::Spin {
                    axis, axis_space, ..
                } = &slot.spec.clip
                {
                    let axis_component_local = match axis_space {
                        crate::object::registry::PartAnimationSpinAxisSpace::Join => {
                            child_anchor_rot * (att.offset.rotation.inverse() * *axis)
                        }
                        crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal => *axis,
                    }
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
        let prompt = build_gen3d_plan_user_text("test", None, Gen3dSpeedMode::Level3);
        assert!(prompt.contains("Placement sanity check"));
        let prompt =
            build_gen3d_plan_user_text_with_hints("test", None, Gen3dSpeedMode::Level3, None, &[]);
        assert!(prompt.contains("Placement sanity check"));
    }

    #[test]
    fn gen3d_review_delta_structural_only_mode_disallows_transform_nudges() {
        let text = build_gen3d_review_delta_system_instructions(false, false, true, 1, 2);
        assert!(text.contains("- appearance_review_enabled: false"));
        assert!(text.contains("Only fix structural issues"));
        assert!(text.contains(
            "Do NOT propose cosmetic-only tweaks, aesthetic regens, or transform nudges."
        ));
        assert!(!text.contains("edit_session: true"));
    }

    #[test]
    fn gen3d_review_delta_edit_session_mode_allows_alignment_tweaks_without_appearance_review() {
        let text = build_gen3d_review_delta_system_instructions(false, true, true, 1, 2);
        assert!(text.contains("- appearance_review_enabled: false"));
        assert!(text.contains("- edit_session: true"));
        assert!(text.contains("Goal: apply the user's requested edits"));
        assert!(text.contains("You MAY propose placement/alignment tweaks"));
        assert!(!text.contains(
            "Do NOT propose cosmetic-only tweaks, aesthetic regens, or transform nudges."
        ));
    }

    #[test]
    fn gen3d_review_delta_round_2_edit_session_is_focused_and_avoids_micro_tweaks() {
        let text = build_gen3d_review_delta_system_instructions(false, true, true, 2, 2);
        assert!(text.contains("Round 2 / focused"), "{text}");
        assert!(
            text.contains("propose ONLY the placement/alignment changes needed"),
            "{text}"
        );
        assert!(
            !text.contains("You MAY propose placement/alignment tweaks"),
            "round 2 should not advertise broad alignment tweaks"
        );
    }

    #[test]
    fn gen3d_review_delta_regen_gate_omits_regen_component_action_when_disallowed() {
        let text = build_gen3d_review_delta_system_instructions(false, false, false, 1, 2);
        assert!(text.contains("regen_component_allowed: false"));
        assert!(text.contains("REGENERATION GATE"));
        assert!(
            !text.contains("\"kind\":\"regen_component\""),
            "regen_component action must be omitted from schema list when disallowed"
        );
        assert!(text.contains("`regen_component` actions are DISALLOWED"));
    }

    #[test]
    fn preserve_mode_plan_prompt_includes_planned_and_actual_sizes() {
        let components = vec![
            Gen3dPlannedComponent {
                display_name: "1. torso".into(),
                name: "torso".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::new(1.0, 2.0, 3.0),
                actual_size: Some(Vec3::new(4.0, 5.0, 6.0)),
                anchors: vec![],
                contacts: vec![],
                attach_to: None,
            },
            Gen3dPlannedComponent {
                display_name: "2. head".into(),
                name: "head".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::new(0.5, 0.5, 0.5),
                actual_size: Some(Vec3::new(1.0, 1.2, 1.4)),
                anchors: vec![],
                contacts: vec![],
                attach_to: Some(super::super::Gen3dPlannedAttachment {
                    parent: "torso".into(),
                    parent_anchor: "head_socket".into(),
                    child_anchor: "torso_socket".into(),
                    offset: Transform::IDENTITY,
                    joint: None,
                    animations: vec![],
                }),
            },
        ];

        let prompt = build_gen3d_plan_user_text_preserve_existing_components(
            "test",
            None,
            Gen3dSpeedMode::Level3,
            None,
            &components,
            "",
            "additive",
            &[],
            None,
        );
        assert!(prompt.contains("planned_size=[1.000,2.000,3.000]"));
        assert!(prompt.contains("actual_size=[4.000,5.000,6.000]"));
    }

    #[test]
    fn gen3d_plan_system_instructions_disallow_component_level_joint_field() {
        let text = build_gen3d_plan_system_instructions();
        assert!(text.contains("`joint` is ONLY allowed inside `attach_to`"));
        assert!(text.contains("Do NOT output a component-level field named \"joint\""));
    }

    #[test]
    fn gen3d_plan_ops_system_instructions_disallow_full_plan_json() {
        let text = build_gen3d_plan_ops_system_instructions();
        assert!(text.contains("Do NOT output a full plan JSON"));
        assert!(text.contains("\"version\":1"));
        assert!(text.contains("\"ops\""));
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
         Collider (IMPORTANT):\n\
         - `collider` is used as the unit's selection circle and click/target hit area.\n\
         - For movable units (`mobility.kind` = `ground` / `air`), you MUST output a top-level `collider`.\n\
         - Size `collider` to the MAIN BODY footprint only.\n\
           - Do NOT inflate it to cover long tails, wings, antennas, swords, barrels, or other protrusions.\n\
           - If the unit is long/segmented (snake, tentacles), pick a reasonable central body footprint.\n\
         - For static objects (`mobility.kind` = `static`), `collider` is optional; if omitted, the engine falls back to a size-based AABB.\n\n\
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
            - Engine constraint (strict): for EVERY attachment, the join anchor axes must be in the same hemisphere.\n\
              - Require: dot(parent_anchor.forward, child_anchor.forward) > 0 AND dot(parent_anchor.up, child_anchor.up) > 0.\n\
                (This dot check is applied as an engine guardrail in component-local coordinates.)\n\
              - If either dot is negative, FIX the anchor bases (flip the child anchor's forward/up as needed) and/or encode the flip via `attach_to.offset` rotation.\n\
                Do NOT rely on opposed anchors.\n\
          - Then `offset.pos[2]` becomes a reliable in/out control along the attachment direction.\n\
          - For flush joins, use a small NEGATIVE `offset.pos[2]` (slight inset/overlap). For surface overlays, use a small POSITIVE `offset.pos[2]` (slight outset) so thin details are not buried.\n\
          Motion metadata (optional; recommended for movable objects):\n\
          - Use `attach_to.joint` to declare articulation (`fixed`/`hinge`/`ball`/`free`).\n\
            - Joint axes/limits are expressed in the PARENT ANCHOR JOIN FRAME (the same frame as `attach_to.offset`).\n\
          - IMPORTANT: `joint` is ONLY allowed inside `attach_to` as `attach_to.joint`.\n\
            - Do NOT output a component-level field named \"joint\".\n\
          - Use `contacts[]` to declare ground contacts (feet/hooves) by referencing one of the component's anchors.\n\
            - For planted contacts, include `contacts[].stance`:\n\
              - `phase_01`: start phase in [0,1).\n\
              - `duty_factor_01`: fraction of cycle in (0,1].\n\
          - Optional: top-level `rig.move_cycle_m` defines meters-per-cycle for locomotion.\n\n\
         Reuse groups (IMPORTANT for speed + consistency):\n\
         - If multiple components should share the SAME geometry (wheels, repeated legs, mirrored parts, numbered sets like `leg_0..leg_7`), declare `reuse_groups`.\n\
         - `reuse_groups` is ONLY an optimization for how geometry is generated. It does NOT declare components.\n\
         - Every `reuse_groups[].source` and every name in `reuse_groups[].targets[]` MUST ALSO appear as a component in `components[]`.\n\
         - If a component name is referenced anywhere (`attach_to.parent`, `aim.components`, `attack.muzzle.component`, `reuse_groups`), it MUST exist in `components[]`.\n\
         - The engine can then generate only the unique components + the declared reuse sources, and fill the remaining targets via deterministic copy.\n\
         - This does NOT change the attachment tree; it only affects how missing geometry gets produced.\n\
         - Reuse targets are allowed (and expected) to differ in per-target `attach_to.offset` (radial/mirrored placement).\n\
         - `reuse_groups[].alignment_frame` (optional; default `join`):\n\
           - `join` (default): alignment accounts for each component's `attach_to.offset` rotation.\n\
           - `child_anchor`: alignment ignores `attach_to.offset` (use when targets are rotated via offset, e.g. reuse a front window on side walls).\n\
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
            \"collider\": {{\"kind\":\"aabb_xz\",\"half_extents\":[x,z]}} | {{\"kind\":\"circle_xz\",\"radius\":r}} | {{\"kind\":\"none\"}} (REQUIRED for mobility ground/air; optional for static),\n\
            \"assembly_notes\": \"...\" (optional),\n\
          \"root_component\": \"component_name\" (optional; otherwise inferred as the only component without attach_to),\n\
          \"reuse_groups\": [\n\
              {{ \"kind\": \"component\" | \"subtree\", \"source\": \"component_name\", \"targets\": [\"component_name\", ...], \"alignment\": \"rotation\" | \"mirror_mount_x\", \"alignment_frame\": \"join\" | \"child_anchor\" (optional), \"mode\": \"detached\" | \"linked\" (optional), \"anchors\": \"preserve_interfaces\" | \"preserve_target\" | \"copy_source\" (optional) }}\n\
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
         - Keep `assembly_notes` short (<= 500 chars). Keep `purpose` short; omit `modeling_notes` unless essential.\n\
         - Order components from most important (core volumes) to least important.\n\
         - `size` is the component's approximate full bounds extents in the component's LOCAL axes.\n\
         - Keep names simple; no spaces.\n\
         - Output only the JSON object; no markdown.\n"
    )
}

pub(super) fn build_gen3d_plan_ops_system_instructions() -> String {
    "You are a 3D modeling assistant.\n\
Return STRICT JSON for a PlanOps patch.\n\n\
Output format:\n\
- Return ONLY one JSON object matching schema `gen3d_plan_ops_v1`.\n\
- Do NOT output a full plan JSON. Output ONLY: {\"version\":1,\"ops\":[ ... ]}.\n\
- Field names matter:\n\
  - `add_component` uses `name` for the new component name (NOT `component`).\n\
- Prefer the smallest valid patch (minimal ops; avoid touching unrelated components/fields).\n\
"
    .to_string()
}

pub(super) fn build_gen3d_draft_ops_system_instructions() -> String {
    "You are a 3D modeling assistant.\n\
Return STRICT JSON for a DraftOps suggestion list.\n\n\
Output format:\n\
- Return ONLY one JSON object matching schema `gen3d_draft_ops_v1`.\n\
- Output ONLY: {\"version\":1,\"ops\":[ ... ]}.\n\
- Do NOT output markdown or extra commentary.\n\n\
Hard rules:\n\
- Use ONLY component names and part_id_uuid values present in the provided snapshots.\n\
- Do NOT invent part_id_uuid for updates/removals.\n\
- Only add new primitives when necessary; prefer updating existing primitives (transform/color/mesh params).\n\
- Unless allow_remove_parts=true, do NOT output `remove_primitive_part`.\n"
    .to_string()
}

pub(super) fn build_gen3d_draft_ops_user_text(
    edit_prompt: &str,
    image_object_summary: Option<&str>,
    run_id: &str,
    attempt: u32,
    pass: u32,
    plan_hash: &str,
    assembly_rev: u32,
    strategy: &str,
    max_ops: usize,
    allow_remove_parts: bool,
    scene_graph_summary: &serde_json::Value,
    component_parts_snapshots: &[serde_json::Value],
    scope_components: &[String],
) -> String {
    let max_ops = max_ops.clamp(1, 64);

    let mut out = String::new();
    out.push_str(
        "EDIT MODE (DraftOps suggestions): Suggest `apply_draft_ops_v1` ops to modify an existing Gen3D draft IN-PLACE.\n\
Goal: satisfy the user edit request with minimal safe primitive edits.\n\
Do NOT regenerate components.\n\n",
    );

    out.push_str("Request:\n");
    out.push_str("- edit_prompt: ");
    out.push_str(edit_prompt.trim());
    out.push('\n');

    out.push_str("\nRun context:\n");
    out.push_str(&format!(
        "- run_id: {run_id}\n- attempt: {attempt}\n- pass: {pass}\n- plan_hash: {plan_hash}\n- assembly_rev: {assembly_rev}\n"
    ));

    out.push_str("\nGuards:\n");
    out.push_str(&format!("- max_ops: {max_ops}\n"));
    out.push_str(&format!("- strategy: {strategy}\n"));
    out.push_str(&format!(
        "- allow_remove_parts: {}\n",
        if allow_remove_parts { "true" } else { "false" }
    ));
    if scope_components.is_empty() {
        out.push_str("- scope_components: (none)\n");
    } else {
        out.push_str("- scope_components: ");
        for (idx, name) in scope_components.iter().take(32).enumerate() {
            if idx > 0 {
                out.push_str(", ");
            }
            out.push_str(name.trim());
        }
        if scope_components.len() > 32 {
            out.push_str(", …");
        }
        out.push('\n');
    }
    out.push_str(
        "- IMPORTANT: Only use part_id_uuid values that appear in the snapshots below.\n\
- Output MUST be exactly one JSON object.\n",
    );

    out.push_str("\nEffective user prompt context:\n");
    out.push_str(&build_gen3d_effective_user_prompt(
        edit_prompt,
        image_object_summary,
    ));

    out.push_str("\nScene graph summary (JSON):\n");
    out.push_str(
        &serde_json::to_string(scene_graph_summary)
            .unwrap_or_else(|_| scene_graph_summary.to_string()),
    );
    out.push('\n');

    out.push_str("\nComponent parts snapshots (JSON; includes part_id_uuid + recipes):\n");
    for snap in component_parts_snapshots.iter().take(16) {
        out.push_str(&serde_json::to_string(snap).unwrap_or_else(|_| snap.to_string()));
        out.push('\n');
    }
    if component_parts_snapshots.len() > 16 {
        out.push_str("(truncated: too many component parts snapshots)\n");
    }

    out
}

pub(super) fn build_gen3d_plan_ops_user_text_preserve_existing_components(
    raw_prompt: &str,
    image_object_summary: Option<&str>,
    speed: Gen3dSpeedMode,
    style_hint: Option<&str>,
    existing_components: &[Gen3dPlannedComponent],
    existing_assembly_notes: &str,
    preserve_edit_policy: &str,
    rewire_components: &[String],
    plan_template: Option<&serde_json::Value>,
    scope_components: &[String],
    max_ops: usize,
) -> String {
    let max_ops = max_ops.clamp(1, 64);

    let mut out = String::new();
    out.push_str(
        "EDIT MODE (diff-first patch): You are patching an ALREADY-GENERATED Gen3D plan using PlanOps.\n\
Hard requirements:\n\
- Keep ALL existing component names; do NOT rename or delete existing components.\n\
- Preserve existing geometry: avoid requiring regeneration of existing components unless strictly necessary.\n\
- Keep mobility/attack/aim/collider unchanged unless the user request requires changing behavior.\n\
- Output ONLY a single JSON object: {\"version\":1,\"ops\":[...]}.\n\
- Do NOT output the full plan JSON.\n\
",
    );

    out.push_str("\nPreserve-mode edit policy:\n- preserve_edit_policy: ");
    out.push_str(preserve_edit_policy.trim());
    out.push('\n');
    if preserve_edit_policy.trim() == "allow_rewire" {
        out.push_str("- rewire_components (explicit allow-list): ");
        if rewire_components.is_empty() {
            out.push_str("(none)\n");
        } else {
            for (idx, name) in rewire_components.iter().take(24).enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(name.trim());
            }
            if rewire_components.len() > 24 {
                out.push_str(", …");
            }
            out.push('\n');
        }
    }

    out.push_str("\nScope enforcement:\n");
    if scope_components.is_empty() {
        out.push_str("- scope_components: (none; you may touch any existing component)\n");
    } else {
        out.push_str("- scope_components (existing components allow-list): ");
        let mut first = true;
        for name in scope_components.iter().take(32) {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            if !first {
                out.push_str(", ");
            }
            first = false;
            out.push_str(name);
        }
        if scope_components.len() > 32 {
            out.push_str(", …");
        }
        out.push('\n');
        out.push_str("- IMPORTANT: Do NOT output ops that touch existing components outside scope_components.\n");
        out.push_str("- You MAY add new components (new names are allowed).\n");
    }

    out.push_str(&format!(
        "\nHard cap: at most {} total components after patch.\n",
        super::max_components_for_speed(speed)
    ));
    out.push_str(&format!("- Output at most {max_ops} ops.\n"));

    if !existing_assembly_notes.trim().is_empty() {
        out.push_str("\nExisting assembly_notes (context; keep consistent unless needed):\n");
        out.push_str(existing_assembly_notes.trim());
        out.push('\n');
    }

    out.push_str("\nExisting component snapshot (names + interfaces):\n");
    out.push_str("Note: some seeded components can have `actual_size` != `planned_size`. When adding new anchors/components, use `actual_size` for scale.\n");
    if existing_components.is_empty() {
        out.push_str("- (none)\n");
    } else {
        for comp in existing_components
            .iter()
            .take(GEN3D_MAX_COMPONENTS.min(64))
        {
            let is_root = comp.attach_to.is_none();
            out.push_str("- ");
            out.push_str(comp.name.as_str());
            if is_root {
                out.push_str(" (root)");
            } else if let Some(att) = comp.attach_to.as_ref() {
                let t = att.offset.translation;
                let r = att.offset.rotation;
                let s = att.offset.scale;
                out.push_str(&format!(
                    " attach_to parent={} parent_anchor={} child_anchor={} offset.pos=[{:.6},{:.6},{:.6}] offset.rot_quat_xyzw=[{:.6},{:.6},{:.6},{:.6}] offset.scale=[{:.6},{:.6},{:.6}]",
                    att.parent,
                    att.parent_anchor,
                    att.child_anchor,
                    t.x,
                    t.y,
                    t.z,
                    r.x,
                    r.y,
                    r.z,
                    r.w,
                    s.x,
                    s.y,
                    s.z
                ));
                if let Some(joint) = att.joint.as_ref() {
                    let kind = match joint.kind {
                        AiJointKindJson::Fixed => "fixed",
                        AiJointKindJson::Hinge => "hinge",
                        AiJointKindJson::Ball => "ball",
                        AiJointKindJson::Free => "free",
                        AiJointKindJson::Unknown => "unknown",
                    };
                    out.push_str(&format!(" joint.kind={kind}"));
                }
            }
            out.push_str(&format!(
                " planned_size=[{:.3},{:.3},{:.3}]",
                comp.planned_size.x, comp.planned_size.y, comp.planned_size.z
            ));
            if let Some(actual) = comp.actual_size {
                out.push_str(&format!(
                    " actual_size=[{:.3},{:.3},{:.3}]",
                    actual.x, actual.y, actual.z
                ));
            }
            out.push('\n');

            if !comp.anchors.is_empty() {
                out.push_str("  anchors: [");
                let mut first = true;
                for a in comp.anchors.iter().take(24) {
                    let name = a.name.as_ref().trim();
                    if name.is_empty() {
                        continue;
                    }
                    if !first {
                        out.push_str(", ");
                    }
                    first = false;
                    out.push_str(name);
                }
                if comp.anchors.len() > 24 {
                    out.push_str(", …");
                }
                out.push_str("]\n");
            }
        }
    }

    if let Some(template) = plan_template {
        out.push_str("\nPlan template (engine-generated; read-only context):\n");
        out.push_str(&serde_json::to_string(template).unwrap_or_else(|_| template.to_string()));
        out.push('\n');
    }

    out.push_str("\n---\n\n");

    out.push_str(
        "PlanOps patching:\n\
- Return ONLY: {\"version\":1,\"ops\":[ ... ]}.\n\
- Prefer minimal ops; avoid restating or replacing large plan structures.\n\
- Allowed PlanOp.kind values:\n\
  - add_component\n\
  - remove_component\n\
  - set_attach_to\n\
  - set_anchor\n\
  - set_aim_components\n\
  - set_attack_muzzle\n\
  - set_reuse_groups\n\
",
    );
    match preserve_edit_policy.trim() {
        "additive" => {
            out.push_str(
                "Policy details (additive):\n\
- Do NOT rewire existing components (do not change attach_to.parent / parent_anchor / child_anchor).\n\
- Do NOT move existing components (do not change attach_to.offset).\n\
- Only add new components and/or add new anchors.\n\
- For repositioning, use `apply_draft_ops_v1` instead of replanning.\n",
            );
        }
        "allow_offsets" => {
            out.push_str(
                "Policy details (allow_offsets):\n\
- Do NOT rewire existing components (do not change attach_to.parent / parent_anchor / child_anchor).\n\
- You MAY change attach_to.offset for existing components to adjust placement.\n\
- Only add new components and/or add new anchors.\n",
            );
        }
        "allow_rewire" => {
            out.push_str(
                "Policy details (allow_rewire):\n\
- You MAY rewire ONLY the components named in `rewire_components` (explicit allow-list).\n\
- For all other existing components, do NOT rewire and do NOT change offsets.\n",
            );
        }
        _ => {}
    }

    out.push_str(
        "Join frame reminder:\n\
- For EVERY attachment, the join anchor axes must be in the same hemisphere:\n\
  dot(parent_anchor.forward, child_anchor.forward) > 0 AND dot(parent_anchor.up, child_anchor.up) > 0.\n\
- If you need a flip, encode it via `attach_to.offset` rotation; do NOT rely on opposed anchors.\n\
- If you author any `attach_to.offset` rotation (`forward`/`up` or `rot_quat_xyzw`), you MUST set `attach_to.offset.rot_frame` explicitly (`join` or `parent`) or the engine will reject the plan.\n",
    );

    out.push_str(&format!("Speed mode: {}.\n", speed.label()));

    if let Some(style) = style_hint.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        out.push_str("Additional style preference (use this unless the user notes forbid it): ");
        out.push_str(style);
        out.push('\n');
    }
    out.push_str(&build_gen3d_effective_user_prompt(
        raw_prompt,
        image_object_summary,
    ));
    out
}

pub(super) fn build_gen3d_component_system_instructions() -> String {
    format!(
        "You are a 3D modeling assistant.\n\
         Return STRICT JSON for a single component.\n\n\
         You may receive up to 2 user reference images (optional). If present, use them to match the component's shape and proportions.\n\n\
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
         - Primitive axis reminder: for `cylinder` and `cone`, the mesh's length/height axis is the part's local +Y axis (so use `scale.y` for length/height and aim it by setting `up`).\n\
         - The component's overall local AABB should be close to the plan's `target_size` per-axis (do not swap/permutate axes).\n\
         - Output only the JSON object; no markdown.\n"
    )
}

pub(super) fn build_gen3d_review_delta_system_instructions(
    review_appearance: bool,
    edit_session: bool,
    regen_allowed: bool,
    review_delta_round_index: u32,
    review_delta_rounds_max: u32,
) -> String {
    let focus_mode = if review_delta_round_index <= 1 {
        "broad"
    } else {
        "main_issue_only"
    };

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
    }
    if edit_session {
        out.push_str("- edit_session: true\n");
    }
    out.push_str(&format!(
        "- review_delta_round: {review_delta_round_index}/{review_delta_rounds_max}\n"
    ));
    out.push_str(&format!("- review_delta_focus: {focus_mode}\n"));
    if focus_mode == "broad" {
        out.push_str(
            "\nTWO-ROUND POLICY (Round 1 / broad):\n\
- Fix ALL objective errors first (smoke/validate severity=\"error\").\n\
- Then satisfy the user's main request.\n\
- Prefer ONE comprehensive, machine-appliable action list. Avoid micro-iterations.\n\
- If no meaningful actions remain, return ONLY {\"kind\":\"accept\"}.\n",
        );
    } else {
        out.push_str(
            "\nTWO-ROUND POLICY (Round 2 / focused):\n\
- Fix any remaining objective errors first (smoke/validate severity=\"error\").\n\
- Then focus ONLY on the main issue (the user's request). Do NOT propose optional improvements or minor nudges.\n\
- If objective errors are cleared and the main issue is satisfied (or cannot be improved deterministically from the structured summaries), return ONLY {\"kind\":\"accept\"}.\n",
        );
    }

    if !review_appearance {
        if edit_session {
            out.push_str(
                "\nEdit-session guidance:\n\
- Goal: apply the user's requested edits to the existing draft.\n\
- Treat user notes as authoritative. If the user says something is misaligned, assume it is and fix it.\n",
            );
            if focus_mode == "broad" {
                out.push_str(
                    "- You MAY propose placement/alignment tweaks (tweak_component_transform / tweak_component_resolved_rot_world / tweak_attachment / tweak_anchor / tweak_contact) even when smoke/validate report ok.\n\
- Still prioritize objective errors first (if any).\n\
- Do NOT propose cosmetic-only changes.\n",
                );
            } else {
                out.push_str(
                    "- In this round, propose ONLY the placement/alignment changes needed for the main issue.\n\
- Still prioritize objective errors first (if any).\n\
- Do NOT propose cosmetic-only changes or exploratory micro-tweaks.\n",
                );
            }
        } else {
            out.push_str(
                "\nStructural-only guidance (appearance_review_enabled=false):\n\
- Only fix structural issues:\n\
  - smoke_results.motion_validation issues with severity=\"error\"\n\
  - other smoke_results issues with severity=\"error\" (if present)\n\
- Do NOT propose cosmetic-only tweaks, aesthetic regens, or transform nudges.\n\
- Ignore smoke severity=\"warn\" issues unless they imply a severity=\"error\".\n",
            );
        }
    }
    if regen_allowed {
        out.push_str("- regen_component_allowed: true\n");
    } else {
        out.push_str("- regen_component_allowed: false\n");
        out.push_str("\nREGENERATION GATE:\n");
        out.push_str("- `regen_component` actions are DISALLOWED in this call. Prefer non-regen tweaks, run `qa_v1` to open the gate when errors exist, or disable preserve mode and rebuild.\n");
    }
    out.push_str(
	        "\nIMPORTANT:\n\
	     - Do NOT assume the engine will \"auto-fix\" placement. If something is wrong, request explicit edits.\n\
	     - Do NOT output Euler angles.\n\
	       - Rotation fields (IMPORTANT):\n\
	         - For `tweak_component_transform.set.rot`, use either:\n\
	           - a basis: {\"forward\":[x,y,z],\"up\":[x,y,z]}\n\
	           - or a quaternion: {\"quat_xyzw\":[x,y,z,w]}\n\
	         - Prefer `tweak_component_resolved_rot_world` when the intent is to set a component's RESOLVED WORLD rotation (e.g. \"make the shin upright in world\").\n\
	           - It takes a WORLD-space rotation basis/quaternion and the engine deterministically solves the required `attach_to.offset.rotation` using the known parent/child anchors.\n\
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
",
    );
    if regen_allowed {
        out.push_str(
            "         {\"kind\":\"regen_component\",\"component_id\":\"<uuid>\",\"updated_modeling_notes\":\"...\" (optional),\"reason\":\"...\" (optional)},\n",
        );
    }
    out.push_str(
        "         {\"kind\":\"tweak_component_transform\",\"component_id\":\"<uuid>\",\"set\":{...} (optional),\"delta\":{...} (optional),\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_component_resolved_rot_world\",\"component_id\":\"<uuid>\",\"rot\":{...},\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_anchor\",\"component_id\":\"<uuid>\",\"anchor_name\":\"name\",\"set\":{...} (optional),\"delta\":{...} (optional),\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_attachment\",\"component_id\":\"<uuid>\",\"set\":{...},\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_contact\",\"component_id\":\"<uuid>\",\"contact_name\":\"name\",\"stance\": null (optional),\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_mobility\",\"mobility\":{...},\"reason\":\"...\" (optional)},\n\
         {\"kind\":\"tweak_attack\",\"attack\":{...},\"reason\":\"...\" (optional)}\n\
       ]\n\
     }\n\n\
     Notes:\n\
     - `tweak_component_transform` edits the component's attachment OFFSET relative to its parent.\n\
       - Its rotation fields (`set.rot` / `delta.rot_quat_xyzw`) rotate the ATTACHMENT OFFSET frame (join frame), not the component's local axes.\n\
       - If you want a target component rotation in WORLD space, use `tweak_component_resolved_rot_world` instead.\n\
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
     - Use `replan` only if the component breakdown or attachment tree is fundamentally wrong.\n",
    );
    if regen_allowed {
        out.push_str(
            "     - Use `regen_component` if the component geometry is structurally wrong (not for tiny details).\n",
        );
    } else {
        out.push_str(
            "     - `regen_component` is DISALLOWED when regen_component_allowed=false.\n",
        );
    }
    out
}

pub(super) fn build_gen3d_review_delta_user_text(
    run_id: &str,
    attempt: u32,
    plan_hash: &str,
    assembly_rev: u32,
    raw_prompt: &str,
    image_object_summary: Option<&str>,
    scene_graph_summary: &serde_json::Value,
    smoke_results: &serde_json::Value,
    review_delta_round_index: u32,
    review_delta_rounds_max: u32,
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

    fn read_vec4(value: &serde_json::Value) -> Option<(f32, f32, f32, f32)> {
        let arr = value.as_array()?;
        if arr.len() != 4 {
            return None;
        }
        Some((
            arr[0].as_f64()? as f32,
            arr[1].as_f64()? as f32,
            arr[2].as_f64()? as f32,
            arr[3].as_f64()? as f32,
        ))
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

    fn fmt_vec4(value: Option<(f32, f32, f32, f32)>) -> String {
        match value {
            Some((x, y, z, w)) => format!("[{:.2},{:.2},{:.2},{:.2}]", x, y, z, w),
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
            let offset_rot_quat_join = c
                .get("attach_to")
                .and_then(|a| a.get("offset"))
                .and_then(|o| o.get("rot_quat_xyzw"))
                .and_then(read_vec4);
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
            let parent_anchor_frame_forward = c
                .get("attach_to")
                .and_then(|a| a.get("parent_anchor_frame"))
                .and_then(|f| f.get("forward"))
                .and_then(read_vec3);
            let parent_anchor_frame_up = c
                .get("attach_to")
                .and_then(|a| a.get("parent_anchor_frame"))
                .and_then(|f| f.get("up"))
                .and_then(read_vec3);
            let child_anchor_frame_forward = c
                .get("attach_to")
                .and_then(|a| a.get("child_anchor_frame"))
                .and_then(|f| f.get("forward"))
                .and_then(read_vec3);
            let child_anchor_frame_up = c
                .get("attach_to")
                .and_then(|a| a.get("child_anchor_frame"))
                .and_then(|f| f.get("up"))
                .and_then(read_vec3);

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
                    " offset.pos(join_frame)={} offset.rot_quat_xyzw(join_frame)={} join_right_world={} join_up_world={} join_forward_world={} parent_anchor_frame.fwd/up(local)={}/{} child_anchor_frame.fwd/up(local)={}/{}",
                    fmt_vec3(offset_pos_join),
                    fmt_vec4(offset_rot_quat_join),
                    fmt_vec3(join_right_world),
                    fmt_vec3(join_up_world),
                    fmt_vec3(join_forward_world),
                    fmt_vec3(parent_anchor_frame_forward),
                    fmt_vec3(parent_anchor_frame_up),
                    fmt_vec3(child_anchor_frame_forward),
                    fmt_vec3(child_anchor_frame_up),
                ));
            }

            out.push_str(&format!(" anchors={:?}\n", anchors));
        }

        out
    }

    let mut out = String::new();
    let focus_mode = if review_delta_round_index <= 1 {
        "broad"
    } else {
        "main_issue_only"
    };
    out.push_str(&format!(
        "Review-delta round: {review_delta_round_index}/{review_delta_rounds_max}\n"
    ));
    out.push_str(&format!("Review-delta focus: {focus_mode}\n"));
    out.push_str("Auto-review pass: propose strict machine-appliable deltas.\n");
    out.push_str(&build_gen3d_effective_user_prompt(
        raw_prompt,
        image_object_summary,
    ));
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
  \"name\": \"1–3 words: a short name for the prefab\",\n\
  \"short\": \"1–2 lines: a concise description (do NOT copy the user prompt)\",\n\
  \"tags\": [\"lower_snake_case\", \"searchable\", \"style\", \"species\", \"category\", \"materials\", \"theme\"]\n\
}\n\n\
Rules:\n\
- `name` must be at most 3 words.\n\
- `name` must be a conclusion based on the provided facts (not just the prompt text).\n\
- `short` must be a conclusion based on the provided facts (not the prompt text).\n\
- Prefer concrete nouns/adjectives; avoid marketing fluff.\n\
- `tags` must be lower_snake_case tokens (no spaces, no hyphens).\n\
- Include tags that help scene building search (style/species/category/theme/material/function).\n\
- Keep tags stable and generic (open vocabulary); avoid internal ids/UUIDs.\n"
        .to_string()
}

pub(super) fn build_gen3d_motion_authoring_system_instructions() -> String {
    "You are the Gravimera Gen3D motion authoring assistant.\n\
You will be given a generated component graph (components + attachments + anchors + current base offsets).\n\
Your job is to author explicit per-edge animation clips.\n\
Return ONLY a single JSON object for gen3d_motion_authoring_v1 (no markdown, no prose).\n\n\
Schema:\n\
{\n\
  \"version\": 1,\n\
  \"applies_to\": {\"run_id\":\"uuid\",\"attempt\":0,\"plan_hash\":\"sha256:...\",\"assembly_rev\":0},\n\
  \"decision\": \"author_clips\" | \"regen_geometry_required\",\n\
  \"reason\": \"short reason\",\n\
  \"replace_channels\": [\"idle\",\"move\",\"attack_primary\"],\n\
  \"edges\": [\n\
    {\n\
      \"component\": \"child_component_name\",\n\
      \"slots\": [\n\
        {\n\
          \"channel\": \"idle|move|attack_primary|ambient\",\n\
          \"driver\": \"always|move_phase|move_distance|attack_time\",\n\
          \"speed_scale\": 1.0,\n\
          \"time_offset_units\": 0.0,\n\
          \"clip\": {\n\
            \"kind\": \"loop|once|ping_pong|spin\",\n\
            \"duration_units\": 1.0,\n\
            \"keyframes\": [\n\
              {\"t_units\": 0.0, \"delta\": {\"pos\": [0,0,0] | null, \"rot_quat_xyzw\": [0,0,0,1] | null, \"scale\": [1,1,1] | null}}\n\
            ]\n\
          }\n\
        }\n\
      ]\n\
    }\n\
  ],\n\
  \"notes\": \"...\" | null\n\
}\n\n\
Rules:\n\
- You MUST copy the provided applies_to values exactly.\n\
- You MUST NOT invent component names. Target attachment edges by naming the CHILD component in `edges[].component`.\n\
  - Do NOT include the root component (it has no parent edge).\n\
- Minimize output size:\n\
  - Only include `edges[]` entries you intend to CHANGE. Omit edges you are not touching.\n\
  - Prefer replacing ONLY the channels you actually author (e.g. if you only author `move`, set replace_channels=[\"move\"]).\n\
  - Prefer a SMALL number of authored edges. Default target: <= 12 edges unless strictly required.\n\
  - Prefer simple loop clips with FEW keyframes (target 3 keyframes at t_units=0.0, 0.5*duration_units, duration_units). Avoid >5 keyframes unless necessary.\n\
  - Prefer using `time_offset_units` to create phase offsets across repeated limbs instead of unique keyframe shapes per limb.\n\
  - Keep deltas small and stable; avoid large translations.\n\
  - Output compact JSON (no pretty formatting).\n\
- Driver units (IMPORTANT):\n\
  - `always` and `attack_time`: time units are seconds.\n\
  - `move_phase` and `move_distance`: time units are meters traveled.\n\
  - `time_offset_units` and `clip.duration_units` are expressed in the SAME units as the driver.\n\
- Coordinate frames:\n\
  - These authored clips animate the ATTACHMENT OFFSET for that edge.\n\
  - JOIN frame axes: +X = join_right, +Y = join_up, +Z = join_forward.\n\
  - For `clip.kind=loop|once|ping_pong`:\n\
    - `delta` transforms are expressed in the PARENT ANCHOR JOIN FRAME (the same frame as `attach_to.offset`).\n\
    - The engine applies: animated_offset = base_offset * delta(t).\n\
    - `rot_quat_xyzw=[x,y,z,w]` is in the JOIN frame; the rotation axis is proportional to `[x,y,z]` (normalized).\n\
    - This does NOT restrict you to axis-aligned rotations; any axis vector in join frame is allowed.\n\
  - For `clip.kind=spin`:\n\
    - You MUST include `axis_space`: \"join\" | \"child_local\".\n\
    - Plain words: \"join\" means \"spin around the joint axes\"; \"child_local\" means \"spin around the model's own axes\".\n\
    - If `axis_space=\"join\"`, `axis` is expressed in the JOIN frame.\n\
    - If `axis_space=\"child_local\"`, `axis` is expressed in the CHILD component's local frame, and the engine rebases it through the child anchor.\n\
- Hinge joints (IMPORTANT):\n\
  - If an edge's attachment joint is `kind=hinge`, `axis_join` is expressed in the JOIN frame.\n\
  - Any authored rotation MUST be a pure twist about `axis_join` (no off-axis swing), or motion validation will fail with `hinge_off_axis`.\n\
  - For hinge edges, prefer `clip.kind=spin` with `axis_space=\"join\"` and `axis` aligned (or anti-aligned) with `axis_join`.\n\
- Fixed joints (diagnostic):\n\
  - If an edge's attachment joint is `kind=fixed`, any authored rotation will trigger a motion validation warning `fixed_joint_rotates`.\n\
    - Only rotate fixed joints if it is intentional, or if the articulation metadata must be updated elsewhere.\n\
- `replace_channels`:\n\
  - If decision=author_clips, list the channels you want the engine to REPLACE on targeted edges before adding your slots.\n\
  - If decision=regen_geometry_required, set replace_channels=[] and edges=[] (do not author clips).\n\
- For movable units, prefer authoring at least `idle` + `move` (and `attack_primary` if the unit has an attack).\n\
- If the prompt implies motion that cannot be achieved with the existing articulation (for example: a snake with only one rigid body component), use decision=regen_geometry_required.\n\
  - In that case, do NOT author clips. Explain what articulation is missing in `reason`/`notes`.\n"
        .to_string()
}

pub(super) fn build_gen3d_motion_authoring_user_text(
    raw_prompt: &str,
    image_object_summary: Option<&str>,
    run_id: &str,
    attempt: u32,
    plan_hash: &str,
    assembly_rev: u32,
    rig_move_cycle_m: Option<f32>,
    has_idle_slot: bool,
    has_move_slot: bool,
    components: &[Gen3dPlannedComponent],
    draft: &Gen3dDraft,
) -> String {
    fn fmt_vec3(v: Vec3) -> String {
        format!("[{:.3},{:.3},{:.3}]", v.x, v.y, v.z)
    }

    fn fmt_quat_xyzw(q: Quat) -> String {
        if !q.is_finite() {
            return "[0,0,0,1]".into();
        }
        let q = q.normalize();
        format!("[{:.4},{:.4},{:.4},{:.4}]", q.x, q.y, q.z, q.w)
    }

    fn anchor_rot_local(component: &Gen3dPlannedComponent, anchor: &str) -> Quat {
        if anchor == "origin" {
            return Quat::IDENTITY;
        }
        component
            .anchors
            .iter()
            .find(|a| a.name.as_ref() == anchor)
            .map(|a| a.transform.rotation)
            .unwrap_or(Quat::IDENTITY)
    }

    fn infer_cycle_m_for_prompt(
        rig_move_cycle_m: Option<f32>,
        components: &[Gen3dPlannedComponent],
    ) -> (f32, &'static str) {
        if let Some(v) = rig_move_cycle_m
            .filter(|v| v.is_finite())
            .map(|v| v.abs())
            .filter(|v| *v > 1e-3)
        {
            return (v, "rig.move_cycle_m");
        }

        for comp in components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            let Some(slot) = att.animations.iter().find(|s| s.channel.as_ref() == "move") else {
                continue;
            };
            if !matches!(
                slot.spec.driver,
                crate::object::registry::PartAnimationDriver::MovePhase
                    | crate::object::registry::PartAnimationDriver::MoveDistance
            ) {
                continue;
            }
            let (duration_secs, repeats) = match &slot.spec.clip {
                crate::object::registry::PartAnimationDef::Loop { duration_secs, .. }
                | crate::object::registry::PartAnimationDef::Once { duration_secs, .. } => {
                    (*duration_secs, 1.0)
                }
                crate::object::registry::PartAnimationDef::PingPong { duration_secs, .. } => {
                    (*duration_secs, 2.0)
                }
                crate::object::registry::PartAnimationDef::Spin { .. } => continue,
            };
            if !duration_secs.is_finite() || duration_secs <= 0.0 {
                continue;
            }
            let speed_scale = slot.spec.speed_scale.max(1e-6);
            let effective = (repeats * duration_secs / speed_scale).abs();
            if effective.is_finite() && effective > 1e-3 {
                return (effective, "move.loop.duration_secs");
            }
        }

        (1.0, "default")
    }

    let mut out = String::new();
    out.push_str("Goal: author explicit per-edge animation clips.\n");
    if let Some(summary) = image_object_summary
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        out.push_str("Reference photo main-object summary (raw images not available):\n");
        out.push_str(summary);
        out.push('\n');
    }
    let prompt = raw_prompt.trim();
    if prompt.is_empty() {
        out.push_str("User prompt: (none)\n");
    } else {
        out.push_str("User prompt:\n");
        out.push_str(prompt);
        out.push('\n');
    }
    out.push('\n');

    out.push_str("You MUST copy these values into `applies_to` exactly:\n");
    out.push_str(&format!("- run_id: {run_id}\n"));
    out.push_str(&format!("- attempt: {attempt}\n"));
    out.push_str(&format!("- plan_hash: {plan_hash}\n"));
    out.push_str(&format!("- assembly_rev: {assembly_rev}\n\n"));

    let mobility_present = draft.root_def().and_then(|r| r.mobility.as_ref()).is_some();
    let attack_present = draft.root_def().and_then(|r| r.attack.as_ref()).is_some();
    out.push_str(&format!(
        "Draft summary: mobility_present={} attack_present={} rig_move_cycle_m={} has_idle_slot={} has_move_slot={}\n",
        mobility_present,
        attack_present,
        rig_move_cycle_m
            .filter(|v| v.is_finite())
            .map(|v| format!("{v:.3}"))
            .unwrap_or_else(|| "null".into()),
        has_idle_slot,
        has_move_slot,
    ));
    out.push('\n');

    out.push_str("Join frame convention (IMPORTANT):\n");
    out.push_str("- The JOIN frame basis vectors are: +X = join_right_world, +Y = join_up_world, +Z = join_forward_world.\n");
    out.push_str("- For `delta.rot_quat_xyzw=[x,y,z,w]`, the rotation axis is proportional to `[x,y,z]` in the JOIN frame (normalized). You can rotate about any join-frame axis (not just the basis axes).\n");
    out.push('\n');

    let mut name_to_idx: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (idx, c) in components.iter().enumerate() {
        name_to_idx.insert(c.name.as_str(), idx);
    }

    let (cycle_m, cycle_source) = infer_cycle_m_for_prompt(rig_move_cycle_m, components);
    let cycle_m = cycle_m.abs().max(1e-3);

    let slip_warn_m: f32 = (0.08 + 0.08 * cycle_m).clamp(0.12, 0.35);
    let slip_error_m: f32 = slip_warn_m * 2.0;
    let lift_warn_m: f32 = (0.06 + 0.06 * cycle_m).clamp(0.10, 0.30);
    let lift_error_m: f32 = lift_warn_m * 2.0;

    out.push_str("Motion validation model (important):\n");
    out.push_str(&format!(
        "- The root is assumed to translate forward along WORLD +Z by cycle_m meters per cycle (cycle_m={cycle_m:.3}, source={cycle_source}).\n"
    ));
    out.push_str("- IMPORTANT: For `driver=move_phase` and `driver=move_distance`, time units are METERS traveled (not seconds).\n");
    out.push_str(&format!(
        "  - To make one `move` loop match this cycle, set `clip.duration_units` ~= cycle_m (or ~= cycle_m * speed_scale if you set speed_scale != 1).\n"
    ));
    out.push_str(&format!(
        "- If a ground contact has a stance schedule, it is treated as PLANTED during stance: keep its anchor stable in world XZ and near-constant Y during stance.\n  - slip_error_m={slip_error_m:.3} (warn={slip_warn_m:.3}) lift_error_m={lift_error_m:.3} (warn={lift_warn_m:.3})\n"
    ));
    out.push_str("- Stance schedule semantics: stance runs from phase_01 (inclusive) for duty_factor_01 of the cycle (wrap at 1.0).\n");
    out.push('\n');

    let mut ground_contact_components: Vec<&str> = Vec::new();
    let mut ground_contact_lines: Vec<String> = Vec::new();
    for comp in components.iter() {
        for contact in comp.contacts.iter() {
            if contact.kind != AiContactKindJson::Ground {
                continue;
            }
            ground_contact_components.push(comp.name.as_str());
            let stance = contact
                .stance
                .as_ref()
                .map(|s| {
                    format!(
                        "{{phase_01={:.3},duty_factor_01={:.3}}}",
                        s.phase_01, s.duty_factor_01
                    )
                })
                .unwrap_or_else(|| "null".to_string());
            let anchor_pos_local = comp
                .anchors
                .iter()
                .find(|a| a.name.as_ref() == contact.anchor.trim())
                .map(|a| fmt_vec3(a.transform.translation))
                .unwrap_or_else(|| "null".to_string());
            ground_contact_lines.push(format!(
                "- component={} contact={} anchor={} anchor_pos_local={} stance={}",
                comp.name.trim(),
                contact.name.trim(),
                contact.anchor.trim(),
                anchor_pos_local,
                stance
            ));
        }
    }
    ground_contact_components.sort();
    ground_contact_components.dedup();
    ground_contact_lines.sort();
    ground_contact_lines.dedup();

    if ground_contact_lines.is_empty() {
        out.push_str("Ground contacts: (none)\n\n");
    } else {
        let total_ground_contacts = ground_contact_lines.len();
        out.push_str("Ground contacts:\n");
        for line in ground_contact_lines.iter().take(64) {
            out.push_str(line);
            out.push('\n');
        }
        if total_ground_contacts > 64 {
            out.push_str(&format!(
                "... (truncated; total_ground_contacts={})\n",
                total_ground_contacts
            ));
        }
        out.push('\n');

        out.push_str("Contact parent chains (ground contact component <- parent <- ...):\n");
        for &name in ground_contact_components.iter().take(64) {
            let mut chain: Vec<&str> = Vec::new();
            let mut cursor = name;
            for _ in 0..64 {
                chain.push(cursor);
                let Some(idx) = name_to_idx.get(cursor).copied() else {
                    break;
                };
                let Some(att) = components.get(idx).and_then(|c| c.attach_to.as_ref()) else {
                    break;
                };
                cursor = att.parent.as_str();
            }
            out.push_str(&format!("- {}\n", chain.join(" <- ")));
        }
        if ground_contact_components.len() > 64 {
            out.push_str(&format!(
                "... (truncated; total_ground_contact_components={})\n",
                ground_contact_components.len()
            ));
        }
        out.push('\n');
        out.push_str("Authoring guidance: prioritize authoring `move` clips on edges in the contact chains above. Use time_offset_units for phase staggering; keep keyframes minimal.\n\n");
    }

    out.push_str("Attachment edges (child components with attach_to):\n");
    for child in components.iter() {
        let Some(att) = child.attach_to.as_ref() else {
            continue;
        };
        let parent_idx = name_to_idx.get(att.parent.as_str()).copied();
        let parent = parent_idx.and_then(|idx| components.get(idx));

        let parent_rot = parent.map(|p| p.rot).unwrap_or(Quat::IDENTITY);
        let parent_anchor_rot_local = parent
            .map(|p| anchor_rot_local(p, att.parent_anchor.as_str()))
            .unwrap_or(Quat::IDENTITY);
        let join_rot_world = parent_rot * parent_anchor_rot_local;
        let join_right_world = join_rot_world * Vec3::X;
        let join_up_world = join_rot_world * Vec3::Y;
        let join_forward_world = join_rot_world * Vec3::Z;

        let joint_summary = att
            .joint
            .as_ref()
            .map(|j| {
                let kind = match j.kind {
                    AiJointKindJson::Fixed => "fixed",
                    AiJointKindJson::Hinge => "hinge",
                    AiJointKindJson::Ball => "ball",
                    AiJointKindJson::Free => "free",
                    AiJointKindJson::Unknown => "unknown",
                };
                let mut out = format!("{{kind={kind}");
                if let Some(axis) = j.axis_join {
                    out.push_str(&format!(
                        ",axis_join={}",
                        fmt_vec3(Vec3::new(axis[0], axis[1], axis[2]))
                    ));
                } else {
                    out.push_str(",axis_join=null");
                }
                if let Some([min_deg, max_deg]) = j.limits_degrees {
                    out.push_str(&format!(",limits_degrees=[{min_deg:.1},{max_deg:.1}]"));
                }
                if let Some([min_deg, max_deg]) = j.swing_limits_degrees {
                    out.push_str(&format!(
                        ",swing_limits_degrees=[{min_deg:.1},{max_deg:.1}]"
                    ));
                }
                if let Some([min_deg, max_deg]) = j.twist_limits_degrees {
                    out.push_str(&format!(
                        ",twist_limits_degrees=[{min_deg:.1},{max_deg:.1}]"
                    ));
                }
                out.push('}');
                out
            })
            .unwrap_or_else(|| "null".to_string());

        let slots: Vec<String> = att
            .animations
            .iter()
            .map(|slot| {
                let kind = match &slot.spec.clip {
                    crate::object::registry::PartAnimationDef::Loop { .. } => "loop",
                    crate::object::registry::PartAnimationDef::Once { .. } => "once",
                    crate::object::registry::PartAnimationDef::PingPong { .. } => "ping_pong",
                    crate::object::registry::PartAnimationDef::Spin { .. } => "spin",
                };
                format!(
                    "{}:{}:{}",
                    slot.channel.as_ref(),
                    format!("{:?}", slot.spec.driver).to_ascii_lowercase(),
                    kind
                )
            })
            .collect();

        out.push_str(&format!(
            "- child={} parent={} parent_anchor={} child_anchor={} base_offset.pos(join)={} base_offset.rot_quat_xyzw(join)={} joint={} join_right_world={} join_up_world={} join_forward_world={} existing_slots={:?}",
            child.name.trim(),
            att.parent.trim(),
            att.parent_anchor.trim(),
            att.child_anchor.trim(),
            fmt_vec3(att.offset.translation),
            fmt_quat_xyzw(att.offset.rotation),
            joint_summary,
            fmt_vec3(join_right_world),
            fmt_vec3(join_up_world),
            fmt_vec3(join_forward_world),
            slots,
        ));

        let ground_contacts: Vec<String> = child
            .contacts
            .iter()
            .filter(|c| c.kind == AiContactKindJson::Ground)
            .map(|c| {
                let stance = match c.stance.as_ref() {
                    Some(stance) => format!("{:.3}/{:.3}", stance.phase_01, stance.duty_factor_01),
                    None => "null".to_string(),
                };
                format!(
                    "{{contact={},anchor={},stance={}}}",
                    c.name.trim(),
                    c.anchor.trim(),
                    stance
                )
            })
            .collect();
        if !ground_contacts.is_empty() {
            out.push_str(&format!(" ground_contacts={:?}", ground_contacts));
        }
        out.push('\n');
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
