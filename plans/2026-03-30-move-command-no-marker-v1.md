# Investigate Right-Click Move With No Marker On Left Side

## Objective

Identify why right-clicking certain terrain regions yields no green move marker and no movement response, while other regions work normally. Produce a concrete fix plan that preserves terrain-height grounding while ensuring move commands always produce a marker (or a clear failure reason) when clicking valid terrain.

## Implementation Plan

- [ ] Reproduce the issue and instrument the move command entry points to determine which early-return path is triggering. Focus on the guard clauses in `src/rts.rs:754-868` for right-click handling, UI capture, and `cursor_surface_pick` failures, and add temporary logging or debug markers to record which branch exits.
- [ ] Verify whether `cursor_surface_pick` fails on the left-side clicks by inspecting the ray/plane intersection behavior in `src/cursor_pick.rs:17-86`. The current logic only intersects the y=0 plane; if the ray does not intersect in front of the camera or is near-parallel, it returns None. Capture the ray direction and the computed `t_ground` to confirm this is the failure mode.
- [ ] If cursor pick is the failure, decide on a more reliable pick strategy for terrain. Options include raycasting the actual terrain mesh, or using the height sampling function with an XZ intersection derived from a different reference plane when the y=0 intersection is invalid. Document the chosen approach and update the pick flow accordingly.
- [ ] If cursor pick succeeds but no marker appears, inspect the pathing stage in `src/rts.rs:870-945` and `src/navigation.rs:129-318` to see why `find_path_height_aware` returns None. Log the start/goal clamp, `blocked_at` decisions, and grid bounds to determine whether obstacles or bounds are rejecting the path.
- [ ] Confirm that the “build-mode right-click quick-remove” guard is not incorrectly triggering for these clicks by validating the object hit logic at `src/rts.rs:786-801`. If it is, tighten the condition or separate move vs. remove behavior so right-click still yields a move target.
- [ ] Add a small regression test or debug-only validation to ensure right-clicking any visible terrain cell returns a pick and produces a marker (or a specific “blocked” outcome) rather than silently returning. Capture the behavior in a test or a debug-only assertion linked to `src/rts.rs:754-967` and `src/cursor_pick.rs:17-86`.

## Verification Criteria

- Right-clicking on the previously failing left-side region now consistently produces a green move marker and a move order, or a clearly visible “blocked” reason if the target is invalid.
- Cursor pick no longer returns None for valid terrain clicks in the affected region.
- Pathing failures, when they occur, are explained by obstacles or bounds and are surfaced with clear debug evidence during investigation.

## Potential Risks and Mitigations

1. **Risk: Switching to mesh raycast could be more expensive or introduce new failure modes.**
   Mitigation: Keep the current height sampling as the primary source of ground height and only use mesh raycast for stable XZ intersection; benchmark in the problematic scene.

2. **Risk: Fixing cursor pick masks a deeper navigation bound issue.**
   Mitigation: Add instrumentation to differentiate pick failure vs. path failure before committing to a fix.

3. **Risk: Changing right-click behavior in Build mode impacts quick-remove workflows.**
   Mitigation: Guard any behavior changes behind Build/Play checks and add a small regression test to validate both actions.

## Alternative Approaches

1. Allow move markers even when pathing fails, but render them in a “blocked” color and avoid issuing a move order; this makes failures visible without changing path rules.
2. Add a developer toggle to render the nav grid and blocked cells so the invalid left-side area is visually explained during debugging.
