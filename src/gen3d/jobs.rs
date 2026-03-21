use bevy::prelude::*;
use uuid::Uuid;

use super::ai::Gen3dAiJob;
use super::state::{Gen3dDraft, Gen3dPreview, Gen3dWorkshop};
use super::tool_feedback::Gen3dToolFeedbackHistory;

pub(crate) const GEN3D_MAX_RUNNING_JOBS: usize = 3;

pub(crate) struct Gen3dJobContext {
    pub(crate) job_id: Uuid,
    pub(crate) created_at_ms: u128,
    pub(crate) realm_id: String,
    pub(crate) scene_id: String,
    pub(crate) workshop: Gen3dWorkshop,
    pub(crate) draft: Gen3dDraft,
    pub(crate) preview: Gen3dPreview,
    pub(crate) feedback_history: Gen3dToolFeedbackHistory,
    pub(crate) ai_job: Gen3dAiJob,
    pub(crate) auto_save_handled_run_id: Option<Uuid>,
}

impl Gen3dJobContext {
    pub(crate) fn run_id(&self) -> Option<Uuid> {
        self.ai_job.run_id()
    }

    pub(crate) fn is_running(&self) -> bool {
        self.ai_job.is_running()
    }

    pub(crate) fn requires_render_context(&self) -> bool {
        self.ai_job.requires_render_context()
    }

    pub(crate) fn is_empty_session(&self) -> bool {
        self.run_id().is_none()
            && !self.is_running()
            && !self.ai_job.can_resume()
            && self.workshop.images.is_empty()
            && self.workshop.prompt.trim().is_empty()
            && self.draft.defs.is_empty()
    }
}

struct LoadedJobMeta {
    job_id: Uuid,
    created_at_ms: u128,
    realm_id: String,
    scene_id: String,
    auto_save_handled_run_id: Option<Uuid>,
}

impl LoadedJobMeta {
    fn new_empty(realm_id: &str, scene_id: &str) -> Self {
        Self {
            job_id: Uuid::new_v4(),
            created_at_ms: now_ms(),
            realm_id: realm_id.to_string(),
            scene_id: scene_id.to_string(),
            auto_save_handled_run_id: None,
        }
    }
}

#[derive(Resource)]
pub(crate) struct Gen3dJobManager {
    loaded: LoadedJobMeta,
    inactive: Vec<Gen3dJobContext>,
}

impl Default for Gen3dJobManager {
    fn default() -> Self {
        Self {
            loaded: LoadedJobMeta::new_empty("", ""),
            inactive: Vec::new(),
        }
    }
}

impl Gen3dJobManager {
    pub(crate) fn loaded_job_id(&self) -> Uuid {
        self.loaded.job_id
    }

    pub(crate) fn loaded_created_at_ms(&self) -> u128 {
        self.loaded.created_at_ms
    }

    pub(crate) fn inactive_jobs(&self) -> &[Gen3dJobContext] {
        &self.inactive
    }

    pub(crate) fn inactive_jobs_mut(&mut self) -> &mut Vec<Gen3dJobContext> {
        &mut self.inactive
    }

    pub(crate) fn running_jobs_count_including_loaded(&self, loaded_job: &Gen3dAiJob) -> usize {
        let loaded_running = usize::from(loaded_job.is_running());
        let inactive_running = self.inactive.iter().filter(|ctx| ctx.is_running()).count();
        loaded_running.saturating_add(inactive_running)
    }

    pub(crate) fn store_loaded_job_if_non_empty(
        &mut self,
        workshop: &mut Gen3dWorkshop,
        draft: &mut Gen3dDraft,
        preview: &mut Gen3dPreview,
        feedback_history: &mut Gen3dToolFeedbackHistory,
        ai_job: &mut Gen3dAiJob,
    ) {
        let should_store = {
            let has_run_id = ai_job.run_id().is_some();
            let has_context = ai_job.is_running() || ai_job.can_resume();
            let has_prompt = !workshop.prompt.trim().is_empty();
            let has_images = !workshop.images.is_empty();
            let has_draft = !draft.defs.is_empty();
            has_run_id || has_context || has_prompt || has_images || has_draft
        };
        if !should_store {
            return;
        }

        let preview_target = preview.target.clone();
        let preview_camera = preview.camera;
        let preview_root = preview.root;

        let mut ctx = Gen3dJobContext {
            job_id: self.loaded.job_id,
            created_at_ms: self.loaded.created_at_ms,
            realm_id: self.loaded.realm_id.clone(),
            scene_id: self.loaded.scene_id.clone(),
            workshop: std::mem::take(workshop),
            draft: std::mem::take(draft),
            preview: std::mem::take(preview),
            feedback_history: std::mem::take(feedback_history),
            ai_job: std::mem::take(ai_job),
            auto_save_handled_run_id: self.loaded.auto_save_handled_run_id.take(),
        };
        ctx.preview.target = None;
        ctx.preview.camera = None;
        ctx.preview.root = None;
        preview.target = preview_target;
        preview.camera = preview_camera;
        preview.root = preview_root;
        self.inactive.push(ctx);
    }

    pub(crate) fn reset_loaded_job(
        &mut self,
        realm_id: &str,
        scene_id: &str,
        workshop: &mut Gen3dWorkshop,
        draft: &mut Gen3dDraft,
        preview: &mut Gen3dPreview,
        feedback_history: &mut Gen3dToolFeedbackHistory,
        ai_job: &mut Gen3dAiJob,
    ) {
        *workshop = Gen3dWorkshop::default();
        *draft = Gen3dDraft::default();
        let target = preview.target.take();
        let camera = preview.camera;
        let root = preview.root;
        *preview = Gen3dPreview::default();
        preview.target = target;
        preview.camera = camera;
        preview.root = root;
        preview.focus = Vec3::ZERO;
        preview.yaw = super::GEN3D_PREVIEW_DEFAULT_YAW;
        preview.pitch = super::GEN3D_PREVIEW_DEFAULT_PITCH;
        preview.distance = super::GEN3D_PREVIEW_DEFAULT_DISTANCE;
        preview.last_cursor = None;
        preview.show_collision = false;
        preview.collision_dirty = true;
        preview.animation_channel = "idle".to_string();
        preview.animation_channels.clear();
        preview.animation_dropdown_open = false;
        *feedback_history = Gen3dToolFeedbackHistory::default();
        *ai_job = Gen3dAiJob::default();
        self.loaded = LoadedJobMeta::new_empty(realm_id, scene_id);
    }

    pub(crate) fn start_new_loaded_job_session(
        &mut self,
        realm_id: &str,
        scene_id: &str,
        workshop: &mut Gen3dWorkshop,
        draft: &mut Gen3dDraft,
        preview: &mut Gen3dPreview,
        feedback_history: &mut Gen3dToolFeedbackHistory,
        ai_job: &mut Gen3dAiJob,
    ) -> Result<(), ()> {
        if self.running_jobs_count_including_loaded(ai_job) >= GEN3D_MAX_RUNNING_JOBS {
            return Err(());
        }

        self.store_loaded_job_if_non_empty(workshop, draft, preview, feedback_history, ai_job);
        self.reset_loaded_job(
            realm_id,
            scene_id,
            workshop,
            draft,
            preview,
            feedback_history,
            ai_job,
        );
        Ok(())
    }

    pub(crate) fn load_job_by_run_id(
        &mut self,
        run_id: Uuid,
        workshop: &mut Gen3dWorkshop,
        draft: &mut Gen3dDraft,
        preview: &mut Gen3dPreview,
        feedback_history: &mut Gen3dToolFeedbackHistory,
        ai_job: &mut Gen3dAiJob,
    ) -> bool {
        if ai_job.run_id() == Some(run_id) {
            return true;
        }

        let Some(idx) = self
            .inactive
            .iter()
            .position(|ctx| ctx.run_id() == Some(run_id))
        else {
            return false;
        };

        self.store_loaded_job_if_non_empty(workshop, draft, preview, feedback_history, ai_job);

        let preview_target = preview.target.clone();
        let preview_camera = preview.camera;
        let preview_root = preview.root;

        let ctx = self.inactive.swap_remove(idx);
        *workshop = ctx.workshop;
        *draft = ctx.draft;
        *preview = ctx.preview;
        preview.target = preview_target;
        preview.camera = preview_camera;
        preview.root = preview_root;
        *feedback_history = ctx.feedback_history;
        *ai_job = ctx.ai_job;
        self.loaded = LoadedJobMeta {
            job_id: ctx.job_id,
            created_at_ms: ctx.created_at_ms,
            realm_id: ctx.realm_id,
            scene_id: ctx.scene_id,
            auto_save_handled_run_id: ctx.auto_save_handled_run_id,
        };
        true
    }

    pub(crate) fn remove_job_by_run_id(
        &mut self,
        run_id: Uuid,
        workshop: &mut Gen3dWorkshop,
        draft: &mut Gen3dDraft,
        preview: &mut Gen3dPreview,
        feedback_history: &mut Gen3dToolFeedbackHistory,
        ai_job: &mut Gen3dAiJob,
    ) -> bool {
        if ai_job.run_id() == Some(run_id) {
            let realm_id = self.loaded.realm_id.clone();
            let scene_id = self.loaded.scene_id.clone();
            self.reset_loaded_job(
                realm_id.as_str(),
                scene_id.as_str(),
                workshop,
                draft,
                preview,
                feedback_history,
                ai_job,
            );
            return true;
        }

        let before = self.inactive.len();
        self.inactive.retain(|ctx| ctx.run_id() != Some(run_id));
        before != self.inactive.len()
    }

    pub(crate) fn loaded_auto_save_handled_run_id(&self) -> Option<Uuid> {
        self.loaded.auto_save_handled_run_id
    }

    pub(crate) fn set_loaded_auto_save_handled_run_id(&mut self, run_id: Option<Uuid>) {
        self.loaded.auto_save_handled_run_id = run_id;
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u128)
        .unwrap_or(0)
}
