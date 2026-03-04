use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionLogSource {
    Brain,
    Player,
}

#[derive(Debug, Clone)]
pub(crate) struct ActionLogEntry {
    pub(crate) at_secs: f32,
    pub(crate) source: ActionLogSource,
    pub(crate) message: String,
}

#[derive(Resource, Debug)]
pub(crate) struct ActionLogState {
    pub(crate) enabled: bool,
    entries: VecDeque<ActionLogEntry>,
    max_entries: usize,
    version: u64,
}

impl Default for ActionLogState {
    fn default() -> Self {
        Self {
            enabled: true,
            entries: VecDeque::new(),
            max_entries: 240,
            version: 0,
        }
    }
}

impl ActionLogState {
    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn entries(&self) -> &VecDeque<ActionLogEntry> {
        &self.entries
    }

    pub(crate) fn clear(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.entries.clear();
        self.version = self.version.wrapping_add(1);
    }

    pub(crate) fn push(
        &mut self,
        at_secs: f32,
        source: ActionLogSource,
        message: impl Into<String>,
    ) {
        if !self.enabled {
            return;
        }
        if self.max_entries == 0 {
            return;
        }

        let msg = message.into();
        let msg_trimmed = msg.trim();
        if msg_trimmed.is_empty() {
            return;
        }

        let mut msg = if msg_trimmed.len() == msg.len() {
            msg
        } else {
            msg_trimmed.to_string()
        };

        // Hard cap message size to guarantee bounded memory usage.
        const MAX_MESSAGE_BYTES: usize = 280;
        if msg.len() > MAX_MESSAGE_BYTES {
            let mut idx = MAX_MESSAGE_BYTES;
            while idx > 0 && !msg.is_char_boundary(idx) {
                idx = idx.saturating_sub(1);
            }
            msg.truncate(idx);
            msg.push('…');
        }

        while self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(ActionLogEntry {
            at_secs,
            source,
            message: msg,
        });
        self.version = self.version.wrapping_add(1);
    }
}

#[derive(SystemParam)]
pub(crate) struct ActionLogWriter<'w> {
    time: Res<'w, Time>,
    log: ResMut<'w, ActionLogState>,
}

impl ActionLogWriter<'_> {
    pub(crate) fn push(&mut self, source: ActionLogSource, message: impl Into<String>) {
        let at_secs = self.time.elapsed_secs();
        self.log.push(at_secs, source, message);
    }
}
