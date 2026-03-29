use super::super::state::{Gen3dDraft, Gen3dWorkshop};
use super::agent_motion_batch::replay_stored_motion_authoring_for_components;
use super::artifacts::write_gen3d_assembly_snapshot;
use super::convert::{self, ConvertedComponentDef};
use super::Gen3dAiJob;

pub(super) fn apply_regenerated_component(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    idx: usize,
    converted: ConvertedComponentDef,
) -> Result<Vec<String>, String> {
    let component_name = job
        .planned_components
        .get(idx)
        .map(|component| component.name.clone())
        .ok_or_else(|| format!("Internal error: missing planned component for idx={idx}"))?;

    let component_def = converted.def;
    if let Some(component) = job.planned_components.get_mut(idx) {
        component.actual_size = Some(component_def.size);
        component.anchors = component_def.anchors.clone();
        component.articulation_nodes = converted.articulation_nodes;
    }

    let target_id = component_def.object_id;
    if let Some(existing) = draft.defs.iter_mut().find(|def| def.object_id == target_id) {
        *existing = component_def;
    } else {
        draft.defs.push(component_def);
    }

    let root_idx = job
        .planned_components
        .iter()
        .position(|component| component.attach_to.is_none())
        .ok_or_else(|| {
            "Internal error: missing root component for regenerated component.".to_string()
        })?;
    convert::resolve_planned_component_transforms(&mut job.planned_components, root_idx)?;
    convert::sync_attachment_tree_to_defs(&job.planned_components, draft)?;

    let replayed = replay_stored_motion_authoring_for_components(
        workshop,
        job,
        draft,
        std::slice::from_ref(&component_name),
    )?;

    convert::update_root_def_from_planned_components(
        &job.planned_components,
        &job.plan_collider,
        draft,
    );
    write_gen3d_assembly_snapshot(job.step_dir.as_deref(), &job.planned_components);
    job.assembly_rev = job.assembly_rev.saturating_add(1);

    Ok(replayed)
}
