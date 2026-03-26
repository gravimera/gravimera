# Object Forms and Transformations (v1)

This document specifies a simple, instance-level “multiple forms” feature for objects. A **form** is a reference to a prefab (`ObjectDef`) by id; an object instance can hold multiple forms and switch between them at runtime.

## Goals

- Any eligible object instance can have **N forms** (default `N = 1`).
- A player can switch forms quickly during play/build without an authoring panel.
- Switching forms is **visualized** via an automatic “mechanical transform” animation between primitive parts.
- Forms must **persist** in saves (both `scene.grav` and scene sources pinned instances).

## Definitions

- **Prefab id**: a stable `u128` UUID that identifies an `ObjectDef`.
- **Form list**: an ordered list of prefab ids `[p0, p1, ...]`.
- **Active form**: the currently selected element of the form list.
- **Unit** vs **Building**:
  - A **unit** is a `Commandable` instance whose active prefab has `mobility` present.
  - A **building** is a `BuildObject` instance whose active prefab has `mobility` absent.
  - **Hard rule**: units and buildings cannot transform between each other.

## Runtime State (per instance)

Per object instance, store:

- `forms: Vec<prefab_id>` where `forms.len() >= 1`
- `active: usize` where `active < forms.len()`
- The instance’s `ObjectPrefabId` must equal `forms[active]`.

Default behavior when an instance does not explicitly store forms: treat it as `forms = [ObjectPrefabId]` and `active = 0`.

## Inputs (Build and Play)

- `Tab`: switch *all selected* eligible instances to their next form (`active = (active + 1) % forms.len()`).
  - If `forms.len() == 1`, do nothing.
  - If a form would cross unit/building category, it is skipped (or the switch is blocked).
  - `Shift+Tab` plays the transform animation at 1/10 speed (10x duration).

- Hold `C`: copy current form from a hovered source instance to a set of destination instances.
  - Press and hold `C`: snapshot current selection as the destination set and show a copy cursor indicator.
  - While holding `C`: hover a single source instance; the cursor indicator surrounds the hovered source.
  - Release `C`: for each destination:
    - Append the source’s **current** form prefab id to the destination’s form list (dedupe).
    - Immediately switch destination to the newly appended form.
  - Press `Esc` to cancel.
  - Cross-category copy (unit→building or building→unit) is blocked/skipped.

Build/Play mode toggling must not occupy the `Tab` key.

## Visuals: Mechanical Transform (automatic)

Switching forms is visualized by animating the object’s primitive parts from the old form to the new form:

- Flatten both prefabs to a list of leaf primitives (resolving `ObjectRef` + anchors/attachments).
- Build a deterministic mapping between old primitives and new primitives, preferring same-primitive-type matches.
- Same primitive type: interpolate transform (position/rotation/scale) and color over time.
- Different primitive type: shrink/fade out the old primitive while grow/fade in the new primitive.
- Count mismatch: unmatched primitives shrink/fade out (old) or grow/fade in (new).

This animation is generated at runtime; there is no transformation authoring UI.

## UI Badge

Any instance with more than one form displays a circular badge anchored above the object in screen space showing `i/n` (1-based), e.g. `2/3`.

The badge is always visible (not only when selected).

## Persistence

### `scene.grav`

`scene.grav` instance records must persist:

- `forms[]` list of prefab ids (UUIDs)
- `active` index
- `is_protagonist` boolean flag (Player Character; exactly one instance per scene)

The file format is versioned and may bump version without backward compatibility.

### Scene sources pinned instances (JSON)

Pinned instance JSON documents may include optional fields:

- `forms`: array of prefab UUID strings
- `active_form`: integer index into `forms`

`prefab_id` remains the active prefab id for minimal compatibility.
