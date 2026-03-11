# `motion_metrics_v1`

Read-only Gen3D tool that computes deterministic numeric motion metrics for the current draft.
It is designed to make “bigger stride / smaller stride” and “foot slipping” actionable without guesswork.

## Why this exists

- **Stride becomes measurable**: per-contact step/excursion ranges in the **root frame**.
- **Planted contact quality becomes measurable**: slip + lift during stance in **world space**.
- **No silent mutation**: this tool does not change the draft.

## Args (v1)

Schema:

```json
{ "version": 1, "sample_count": 32 }
```

- `sample_count` is clamped to `[8, 256]` (higher = smoother metrics, slower).

## Output (v1)

Top-level keys:

- `rig_summary`
  - `cycle_m`: inferred move cycle length (sampling domain for `move` driver units).
  - `cycle_source`: where `cycle_m` came from (ex: `rig.move_cycle_m` or `move.loop.duration_secs`).
  - `rig_max_dim_m`: max planned component dimension (scale reference).
  - `contacts_ground_total`, `contacts_ground_with_stance`, `sample_count`, etc.
- `summary`: aggregated stats across contacts
  - `root_frame_forward_range_m`: stride/excursion magnitude summary.
  - `stance_slip_max_m_xz`: planted-contact slip summary.
  - `stance_lift_max_m`: planted-contact lift summary.
- `ground_contacts[]`: per-contact metrics
  - `root_frame.forward_range_m`: anchor excursion along root forward (+Z) over one cycle.
  - `root_frame.forward_range_fraction_of_cycle`: `forward_range_m / cycle_m`.
  - `root_frame.cycle_fraction_of_rig_max_dim`: `cycle_m / rig_max_dim_m` (scale-normalized stride).
  - `stance_metrics` (present only when `contacts[].stance` exists and `move_is_spin=false`):
    - `slip_max_m_xz`: max horizontal drift during stance (world space).
    - `lift_max_m`: max vertical drift during stance (world space).
    - `baseline`: stance baseline phase + world position.

## How to use it for “bigger stride”

Common workflow:

1. Call `motion_metrics_v1`.
2. Pick an explicit numeric goal in the same units as the rig:
   - Example goal (normalized): increase `cycle_fraction_of_rig_max_dim`.
   - Example goal (absolute): increase `root_frame.forward_range_m` for the foot contacts.
3. Apply changes (typically via motion authoring) and re-run `motion_metrics_v1` to confirm.

## Notes

- Wheels/rollers often use a `move` clip of kind `spin`. For those, `move_is_spin=true` and the tool omits `stance_metrics` because a rim anchor is not a planted point during stance.

