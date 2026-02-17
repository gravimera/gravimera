# Controls

Gravimera is a sandbox with multiple modes. Some inputs are mode-dependent.

## Common

- Toggle Build/Play: `Tab`
- Scene Builder panel: click **Scene** (top-left)
- Console: `Enter` (commands: `/easy`, `/hard`, `/hell`)
- Zoom: mouse wheel (in/out; no orbit/rotation)
- Camera pan (mouse): move cursor near the window edge
- Camera pan (keyboard): `W` / `A` / `S` / `D` when nothing is selected
- Camera rotate: `Z` / `X` / `Q` / `E` (Up/Down/Left/Right)
- Direct move (selected units/build objects): `W` / `A` / `S` / `D` (camera-relative; camera follows selection with slack)
- Slow move (selected units): `CapsLock` (toggle; 1/3 speed)

## Selection

- Select: `LMB` click
- Box select: `LMB` drag
- Clear selection: `LMB` click empty space (or box-select empty)

Selection is disabled while:

- Placing build objects in Build mode
- Holding `Space` (fire targeting)

## Build mode

Place objects:

- Choose object: `1` Block, `2` Fence, `3` Tree
- Place: `LMB`
- Fence axis / tree size: `F`
- Exit placement (selection/edit mode): `Esc`

Edit selected build objects (when not placing):

- Delete: `Delete` / `Backspace`
- Move: `W` / `A` / `S` / `D` (camera-relative; hold to repeat)
- Move (grid nudge): arrow keys (`Shift` = bigger step)
- Rotate: `,` / `.` (`Shift` = 45° steps)
- Scale: `-` / `=` (`Shift` = bigger steps)
- Duplicate: `Ctrl/Cmd + D`

Units in Build mode:

- Duplicate selected units: `M`
- Scale selected units: `-` / `=` (`Shift` = bigger steps)

## Play mode

- Move selected: `RMB`
- Fire: hold `Space` to fire at the cursor (point or enemy under cursor)
- Switch weapon: `1/2/3` (when not holding `Shift`)
- Play unit motions: `Shift + 1..9` and `Shift + 0` (slot 10) for selected units
- Restart: `R`

## Gen3D

Enter/exit Gen3D using the in-game **Gen3D** button. Full workflow: `gen_3d.md`.
