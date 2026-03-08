This folder contains ExecPlans (execution plans) for larger features and refactors.

- Active plan: `docs/execplans/execplan_gen3d_codex_json_editing.md`
- Previous Gen3D agent-loop plan (historical): `docs/execplans/execplan_gen3d_codex_style_agent.md`
- Gen3D model export (glTF/GLB + editable bundles): `docs/execplans/execplan_gen3d_export_glb.md`
- Gen3D runtime motion algorithms (rig contracts + realtime switching): `docs/execplans/execplan_gen3d_runtime_motion_algorithms.md`
- Gen3D motion authoring prompt precision + join-frame constraints: `docs/execplans/execplan_gen3d_motion_authoring_prompt_precision_and_plan_join_frames.md`
- Scene generation roadmap: `docs/execplans/execplan_scene_generation_pipeline.md`
- Mechanical transform mapping v2 (grouped assignment for 200+ primitives): `docs/execplans/execplan_object_forms_mechanical_transform_mapping_v2.md`
- Scene storage simplification (remove depot, scene-local prefabs, restart-safe Gen3D edit): `docs/execplans/execplan_scene_local_prefabs_and_self_contained_scene_dat.md`
- Scene generation milestones (execute in order):
  - `docs/execplans/execplan_scene_01_sources_foundation.md`
  - `docs/execplans/execplan_scene_02_sources_roundtrip_automation.md`
  - `docs/execplans/execplan_scene_03_layers_and_compilation.md`
  - `docs/execplans/execplan_scene_04_validation_scorecards.md`
  - `docs/execplans/execplan_scene_05_blueprint_apply_sources.md`
  - `docs/execplans/execplan_scene_06_runs_resume_quality_gate.md`
  - `docs/execplans/execplan_scene_07_procedural_layer_kinds_v1.md`
  - `docs/execplans/execplan_scene_08_human_scene_sources_ui.md`
- Other files here are historical design/implementation notes from earlier iterations. They are kept for reference/debugging.

For new work, create a fresh ExecPlan following `PLANS.md` and keep it up to date while implementing.
