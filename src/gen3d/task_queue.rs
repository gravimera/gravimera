use bevy::prelude::*;
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

use super::ai::Gen3dAiJob;
use super::state::{Gen3dDraft, Gen3dWorkshop};

pub(crate) type Gen3dSessionId = Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gen3dTaskState {
    /// Session exists but no run has been queued.
    Idle,
    Waiting,
    Running,
    Done,
    Failed,
    Canceled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gen3dSessionKind {
    NewBuild,
    EditOverwrite { prefab_id: u128 },
    Fork { prefab_id: u128 },
}

#[derive(Clone, Debug)]
pub(crate) struct Gen3dSessionMeta {
    pub(crate) id: Gen3dSessionId,
    pub(crate) kind: Gen3dSessionKind,
    pub(crate) task_state: Gen3dTaskState,
    pub(crate) created_at_ms: u128,
    pub(crate) updated_at_ms: u128,
}

#[derive(Default)]
pub(crate) struct Gen3dSessionState {
    pub(crate) workshop: Gen3dWorkshop,
    pub(crate) job: Gen3dAiJob,
    pub(crate) draft: Gen3dDraft,
}

fn system_time_ms(time: std::time::SystemTime) -> u128 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u128)
        .unwrap_or(0)
}

fn now_ms() -> u128 {
    system_time_ms(std::time::SystemTime::now())
}

#[derive(Resource)]
pub(crate) struct Gen3dTaskQueue {
    pub(crate) active_session_id: Gen3dSessionId,
    pub(crate) running_session_id: Option<Gen3dSessionId>,
    pub(crate) queue: VecDeque<Gen3dSessionId>,
    pub(crate) metas: HashMap<Gen3dSessionId, Gen3dSessionMeta>,
    pub(crate) inactive_states: HashMap<Gen3dSessionId, Gen3dSessionState>,
}

impl Default for Gen3dTaskQueue {
    fn default() -> Self {
        let id = Uuid::new_v4();
        let now = now_ms();
        let mut metas = HashMap::new();
        metas.insert(
            id,
            Gen3dSessionMeta {
                id,
                kind: Gen3dSessionKind::NewBuild,
                task_state: Gen3dTaskState::Idle,
                created_at_ms: now,
                updated_at_ms: now,
            },
        );
        Self {
            active_session_id: id,
            running_session_id: None,
            queue: VecDeque::new(),
            metas,
            inactive_states: HashMap::new(),
        }
    }
}

impl Gen3dTaskQueue {
    pub(crate) fn active_meta(&self) -> Option<&Gen3dSessionMeta> {
        self.metas.get(&self.active_session_id)
    }

    pub(crate) fn active_meta_mut(&mut self) -> Option<&mut Gen3dSessionMeta> {
        self.metas.get_mut(&self.active_session_id)
    }

    pub(crate) fn ensure_meta(&mut self, id: Gen3dSessionId, kind: Gen3dSessionKind) {
        let now = now_ms();
        self.metas.entry(id).or_insert_with(|| Gen3dSessionMeta {
            id,
            kind,
            task_state: Gen3dTaskState::Idle,
            created_at_ms: now,
            updated_at_ms: now,
        });
    }

    pub(crate) fn set_task_state(&mut self, id: Gen3dSessionId, state: Gen3dTaskState) {
        let now = now_ms();
        if let Some(meta) = self.metas.get_mut(&id) {
            meta.task_state = state;
            meta.updated_at_ms = now;
        }
    }

    pub(crate) fn create_session(
        &mut self,
        kind: Gen3dSessionKind,
        state: Gen3dSessionState,
    ) -> Gen3dSessionId {
        let id = Uuid::new_v4();
        self.ensure_meta(id, kind);
        self.inactive_states.insert(id, state);
        id
    }

    pub(crate) fn swap_active_session(
        &mut self,
        target_id: Gen3dSessionId,
        workshop: &mut Gen3dWorkshop,
        job: &mut Gen3dAiJob,
        draft: &mut Gen3dDraft,
    ) -> Result<(), String> {
        if target_id == self.active_session_id {
            return Ok(());
        }
        if !self.metas.contains_key(&target_id) {
            return Err("Unknown Gen3D session id.".into());
        }

        let current_id = self.active_session_id;
        let current_state = Gen3dSessionState {
            workshop: std::mem::take(workshop),
            job: std::mem::take(job),
            draft: std::mem::take(draft),
        };
        self.inactive_states.insert(current_id, current_state);

        let next_state = self.inactive_states.remove(&target_id).unwrap_or_default();
        *workshop = next_state.workshop;
        *job = next_state.job;
        *draft = next_state.draft;
        self.active_session_id = target_id;
        Ok(())
    }

    pub(crate) fn find_session_for_prefab(&self, prefab_id: u128) -> Option<Gen3dSessionId> {
        self.metas
            .values()
            .find(|meta| match meta.kind {
                Gen3dSessionKind::EditOverwrite { prefab_id: id }
                | Gen3dSessionKind::Fork { prefab_id: id } => id == prefab_id,
                Gen3dSessionKind::NewBuild => false,
            })
            .map(|meta| meta.id)
    }

    pub(crate) fn queue_len(&self) -> usize {
        self.queue.len()
    }
}
