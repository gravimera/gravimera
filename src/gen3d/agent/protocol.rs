use serde::{Deserialize, Serialize};

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

    pub(crate) fn err_with_result(
        call_id: String,
        tool_id: String,
        error: String,
        result: serde_json::Value,
    ) -> Self {
        Self {
            call_id,
            tool_id,
            ok: false,
            result: Some(result),
            error: Some(error),
        }
    }
}
