use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const SCORECARD_FORMAT_VERSION: u32 = 1;
pub(crate) const VALIDATION_REPORT_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct ScorecardScopeV1 {
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) realm_id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) scene_id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) region_filter: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ScorecardSpecV1 {
    pub(crate) format_version: u32,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) scope: ScorecardScopeV1,
    #[serde(default)]
    pub(crate) hard_gates: Vec<HardGateSpecV1>,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) soft_metrics: Vec<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) weights: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind")]
pub(crate) enum HardGateSpecV1 {
    #[serde(rename = "schema")]
    Schema {},
    #[serde(rename = "budget")]
    Budget {
        #[serde(default)]
        max_instances: Option<usize>,
        #[serde(default)]
        max_portals: Option<usize>,
    },
    #[serde(rename = "portals")]
    Portals {
        #[serde(default)]
        require_known_destinations: Option<bool>,
    },
    #[serde(rename = "determinism")]
    Determinism {},
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViolationSeverityV1 {
    Error,
    Warning,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct ViolationEvidenceV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) layer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) local_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prefab_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) portal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) destination_scene_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) marker_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) measured: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) limit: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ValidationViolationV1 {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) severity: ViolationSeverityV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) evidence: Option<ViolationEvidenceV1>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct ProvenanceSummaryV1 {
    pub(crate) pinned_instances: usize,
    pub(crate) instances_by_layer: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ValidationReportV1 {
    pub(crate) format_version: u32,
    pub(crate) tick: u64,
    pub(crate) event_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scene_id: Option<String>,
    pub(crate) hard_gates_passed: bool,
    pub(crate) metrics: BTreeMap<String, serde_json::Value>,
    pub(crate) violations: Vec<ValidationViolationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provenance_summary: Option<ProvenanceSummaryV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fixits: Option<Vec<serde_json::Value>>,
}

impl ValidationReportV1 {
    pub(crate) fn new(scene_id: Option<String>) -> Self {
        Self {
            format_version: VALIDATION_REPORT_FORMAT_VERSION,
            tick: 0,
            event_id: 0,
            scene_id,
            hard_gates_passed: true,
            metrics: BTreeMap::new(),
            violations: Vec::new(),
            provenance_summary: None,
            fixits: None,
        }
    }

    pub(crate) fn push_violation(&mut self, violation: ValidationViolationV1) {
        if matches!(violation.severity, ViolationSeverityV1::Error) {
            self.hard_gates_passed = false;
        }
        self.violations.push(violation);
    }
}
