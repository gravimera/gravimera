use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gen3dAgentStepJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) status_summary: String,
    #[serde(default)]
    pub(crate) actions: Vec<Gen3dAgentActionJsonV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Gen3dAgentActionJsonV1 {
    ToolCall {
        call_id: String,
        tool_id: String,
        #[serde(default)]
        args: serde_json::Value,
    },
    Done {
        #[serde(default)]
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gen3dToolCallJsonV1 {
    pub(crate) call_id: String,
    pub(crate) tool_id: String,
    #[serde(default)]
    pub(crate) args: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gen3dToolResultJsonV1 {
    pub(crate) call_id: String,
    pub(crate) tool_id: String,
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

impl Gen3dToolResultJsonV1 {
    pub(crate) fn ok(call_id: String, tool_id: String, result: serde_json::Value) -> Self {
        Self {
            call_id,
            tool_id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub(crate) fn err(call_id: String, tool_id: String, error: String) -> Self {
        Self {
            call_id,
            tool_id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}
