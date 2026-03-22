See implementation plan: `docs/execplans/todo_reasoning_effort_action_motion_and_plan_images.md`.

- [x] Make the default reasoning effort for all steps `high`.
  - [x] Unite all reasoning-effort configs to a single key: `reasoning_effort`.
- [x] Add another default generated motion for a unit: `action` (operating/handling something important).
- [x] In Gen3D, when users provide images: also send the resolution-handled images to `prompt_intent` and `llm_generate_plan` (agent + pipeline) so planning is more accurate.
