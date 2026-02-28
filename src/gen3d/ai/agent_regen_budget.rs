use crate::config::AppConfig;

use super::Gen3dAiJob;

pub(super) fn ensure_agent_regen_budget_len(job: &mut Gen3dAiJob) {
    let planned_len = job.planned_components.len();
    if job.regen_per_component.len() != planned_len {
        job.regen_per_component.resize(planned_len, 0);
    }
}

pub(super) fn regen_budget_allows(config: &AppConfig, job: &Gen3dAiJob, component_idx: usize) -> bool {
    let max_total = config.gen3d_max_regen_total;
    if max_total > 0 && job.regen_total >= max_total {
        return false;
    }
    let max_per_component = config.gen3d_max_regen_per_component;
    if max_per_component > 0
        && job
            .regen_per_component
            .get(component_idx)
            .copied()
            .unwrap_or(0)
            >= max_per_component
    {
        return false;
    }
    true
}

pub(super) fn consume_regen_budget(config: &AppConfig, job: &mut Gen3dAiJob, component_idx: usize) -> bool {
    ensure_agent_regen_budget_len(job);
    if !regen_budget_allows(config, job, component_idx) {
        return false;
    }
    job.regen_total = job.regen_total.saturating_add(1);
    if component_idx < job.regen_per_component.len() {
        job.regen_per_component[component_idx] =
            job.regen_per_component[component_idx].saturating_add(1);
    }
    true
}

