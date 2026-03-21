use bevy::prelude::Res;

use crate::config::AppConfig;

pub(crate) fn local_world_mutations_allowed(config: Res<AppConfig>) -> bool {
    !(config.automation_enabled && config.automation_monitor_mode)
}
