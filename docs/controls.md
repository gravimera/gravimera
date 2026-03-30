# Controls

Gravimera is a sandbox with multiple modes. Some inputs are mode-dependent.

## Common

- Toggle Build/Play: `F1` (or the top-left **Build/Play** button)
- Entering Build mode restores unit health and revives dead units.
- Cycle forms (selected multi-form objects): `Tab` (`Shift+Tab` plays the transform animation at 1/10 speed)
- Copy current form: hold `C` (captures current selection as destinations), hover a single source object, then release `C` to confirm (press `Esc` to cancel)
- Workspace selector: use the top-left dropdown (**Object Preview** / **Scene Build**) (Build mode only)
- Workspaces are isolated: **Object Preview** saves to `scene.grav`, **Scene Build** saves to `scene.build.grav`, and switching swaps the saved world.
- Camera position/zoom are also isolated per workspace.
- Gen3D workshop: in **Object Preview**, click **Gen3D** (top-left)
- Models panel (hidden by default): in **Object Preview**, click **Show Models** / **Hide Models** (top-left)
- 3D Models panel manage mode: click **Manage** (panel header) to enter multi-select. **Export**/**Delete** + **All**/**None** appear; `Shift`+click selects a range. Click **Done** to exit.
- Scenes panel manage mode: click **Manage** (panel header) to enter multi-select. **Import** stays available, and **Export**/**Delete** + **All**/**None** appear. Delete skips the currently active scene.
- Scene package import/export details: `docs/scene_import_export.md`
- Terrain panel manage mode: click **Manage** (panel header) to enter multi-select. **Export**/**Delete** + **All**/**None** appear; `Shift`+click selects a range. The **Default Terrain** row is not selectable. Click **Done** to exit.
- Terrain package import/export details: `docs/terrain_import_export.md`
- Console: `Enter` (commands: `/easy`, `/hard`, `/hell`; cheat: `who's your daddy` (+ optional amount) boosts all commandable units)
- HUD stats (top-right): object count, primitive count, FPS
- Zoom: mouse wheel (in/out; no orbit/rotation; scrolling over **Scenes**/**3D Models**/**Terrain**/**Meta** panels scrolls their lists instead)
- Meta panel Speak: double-click a unit selection circle to open **Meta**, then use **Speak** (voice: `dog`/`cow`/`dragon`, `content` field, `Speak` button). While speaking, a speech bubble appears above that model showing the spoken content.
- Text inputs: the IME candidate/keyboard anchor follows the focused input field.
- Meta panel Player Character: double-click a commandable unit to open **Meta**, then use **Player Character** → **Set as Player Character**. Each scene has exactly one Player Character, and the choice is saved per scene/workspace.
- Camera pan (mouse): move cursor near the window edge
- Camera pan (keyboard): `W` / `A` / `S` / `D` when nothing is selected
- Camera rotate: `Z` / `X` / `Q` / `E` (Up/Down/Left/Right)
- Direct move (selected units/build objects): `W` / `A` / `S` / `D` (camera-relative; camera follows selection horizontally with slack)
- Slow move (selected units): `CapsLock` (toggle; 1/3 speed)

## Selection

- Select: `LMB` click
- Box select: `LMB` drag
- Clear selection: `LMB` click empty space (or box-select empty)

Notes:

- Units and buildings cannot transform between each other.
- Multi-form objects show a circular `i/n` badge above them.
- All selectable objects (units + buildings) have a selection circle.
  - Units: based on their collision radius (Gen3D: AI-authored main-body footprint; long protrusions may extend outside and won’t be clickable/targetable).
  - Buildings: based on their footprint radius (max X/Z half-extent).
- Cursor targeting uses selection circles:
  - Selection circles are centered at the object’s ground ring (bottom).
  - Hovering an object (cursor enters its selection circle) shows an animated selection circle (same pulse style as form copy).
  - If multiple circles overlap, the most “relevant” object is chosen (deepest inside the circle, then closest to center).
  - While box-selecting (dragging with `LMB` held), objects whose selection circles intersect the box show the same animated circle as a preview.
  - Box select selects all objects whose selection circles intersect the box.
- Selected objects render a ground selection ring sized to `1.1x` their selection radius.

Selection is disabled while:

- Placing build objects in Build mode
- Holding `Space` (fire targeting)
- Holding `C` (form copy)

## Build mode

Place objects:

- Choose object: (no default keybinding; old `B`/`F`/`T` shortcuts removed)
- Place: `LMB`
- Fence axis / tree size: `G`
- Exit placement (selection/edit mode): `Esc`

Edit selected build objects (when not placing):

- Delete: `Delete` / `Backspace`
- Drag move (single selected unit/build object): `Alt + LMB drag`
- Move: `W` / `A` / `S` / `D` (camera-relative; hold to repeat)
- Move (grid nudge): arrow keys (`Shift` = bigger step)
- Rotate: `,` / `.` (`Shift` = 45° steps)
- Scale: `-` / `=` (`Shift` = bigger steps)
- Duplicate: `Ctrl/Cmd + D`

Units in Build mode:

- Duplicate selected units: `M`
- Scale selected units: `-` / `=` (`Shift` = bigger steps)
- Play selected motions (units/build objects): `1..9` and `0` (slot 10)

## Play mode

- Move selected: `RMB`
- Fire: hold `Space` to fire at the cursor (point or enemy under cursor)
- Action Log panel: use the top-left **Log: On/Off** button (Play mode only)
- Enemies do not spawn automatically.
- Switch weapon: `Ctrl/Cmd + 1/2/3`
- Play selected motions (units/build objects): `1..9` and `0` (slot 10)
- Restart: `R`

## Build Preview (Gen3D)

Enter/exit the Gen3D workshop from Build mode via the top-left workspace dropdown (**Object Preview**) and the **Gen3D** button. Full workflow: `docs/gen3d/README.md`.

- Preview orbit: `LMB` drag on the preview panel
- Preview zoom: mouse wheel on the preview panel
- Preview pan: with the cursor over the preview, `W` / `A` / `S` / `D` or arrow keys
- Component inspection: move the cursor over a visible preview component to show a frame and info
  card; when parent/child frames overlap, the more specific nested component wins
- Explode inspection: use the preview panel `Inspect` → `Explode` toggle to separate preview
  components, including nested ones, and show their names without modifying the draft; zoom stays
  centered on the exploded assembly while explode is active
