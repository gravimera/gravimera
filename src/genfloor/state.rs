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

#[derive(Clone, Debug)]
pub(crate) struct GenFloorAiUsage {
    pub(crate) total_tokens: u64,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct GenFloorAiResult {
    pub(crate) def: FloorDefV1,
    pub(crate) usage: Option<GenFloorAiUsage>,
    pub(crate) raw_text: String,
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

    pub(crate) fn last_saved_floor_id(&self) -> Option<u128> {
        self.last_saved_floor_id
    }

    pub(crate) fn set_last_saved_floor_id(&mut self, floor_id: Option<u128>) {
        self.last_saved_floor_id = floor_id;
    }
}
