use bevy::prelude::*;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::genfloor::defs::FloorDefV1;
use crate::threaded_result::SharedResult;

#[derive(Resource, Default, Clone)]
pub(crate) struct GenFloorWorkshop {
    pub(crate) prompt: String,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) draft: Option<FloorDefV1>,
}

impl GenFloorWorkshop {
    pub(crate) fn reset_for_new_build(&mut self) {
        self.prompt.clear();
        self.status.clear();
        self.error = None;
        self.draft = None;
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GenFloorAiUsage {
    pub(crate) total_tokens: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct GenFloorAiResult {
    pub(crate) def: FloorDefV1,
    pub(crate) usage: Option<GenFloorAiUsage>,
}

#[derive(Resource, Default)]
pub(crate) struct GenFloorAiJob {
    pub(crate) running: bool,
    pub(crate) cancel_requested: bool,
    pub(crate) cancel_flag: Option<Arc<AtomicBool>>,
    pub(crate) started_at: Option<std::time::Instant>,
    pub(crate) last_run_elapsed: Option<std::time::Duration>,
    pub(crate) run_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) edit_base_floor_id: Option<u128>,
    pub(crate) save_overwrite_floor_id: Option<u128>,
    pub(crate) last_saved_floor_id: Option<u128>,
    pub(crate) shared: Option<SharedResult<GenFloorAiResult, String>>,
}

impl GenFloorAiJob {
    pub(crate) fn run_elapsed(&self) -> Option<std::time::Duration> {
        if self.running {
            self.started_at
                .map(|start| start.elapsed())
                .or(self.last_run_elapsed)
        } else {
            self.last_run_elapsed
        }
    }

    pub(crate) fn edit_base_floor_id(&self) -> Option<u128> {
        self.edit_base_floor_id
    }

    pub(crate) fn set_edit_base_floor_id(&mut self, floor_id: Option<u128>) {
        self.edit_base_floor_id = floor_id;
    }

    pub(crate) fn save_overwrite_floor_id(&self) -> Option<u128> {
        self.save_overwrite_floor_id
    }

    pub(crate) fn set_save_overwrite_floor_id(&mut self, floor_id: Option<u128>) {
        self.save_overwrite_floor_id = floor_id;
    }

    pub(crate) fn set_last_saved_floor_id(&mut self, floor_id: Option<u128>) {
        self.last_saved_floor_id = floor_id;
    }

    pub(crate) fn reset_for_new_build(&mut self) {
        if self.running {
            return;
        }
        self.cancel_requested = false;
        self.cancel_flag = None;
        self.started_at = None;
        self.last_run_elapsed = None;
        self.run_tokens = 0;
        self.edit_base_floor_id = None;
        self.save_overwrite_floor_id = None;
        self.last_saved_floor_id = None;
        self.shared = None;
    }
}
