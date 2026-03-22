[x] gen3d Pipeline mode does not generate motion. Fix it.
[x] Remove Copy, Edit, Fork buttons from the Meta panel. And add a close button in the top-right corner to the Meta panel. 
[x] After double clicking an object, besides opening the Meta panel (if it is a unit), also opening the Prefabs panel with the corresponding prefab item selected and pop the Preview panel (if the object has Prefab).
[x] On the Preview panel (from Prefab panel)
  [x] Add two buttons
    [x] Modify: open gen3d panel to do modification
    [x] Duplicate: copy a new prefab (new id)
  [x] Make the info section taller, to show more text.
[x] On Prefabs panel
  [x]  We support multiple gen3d panels, triggered by different places. But only one gen3d task can really run at the same time. The others are waiting.
  [x] When a prefab item is being edit, mark a working animation on the thumbnail. Click the item to show its current gen3d panel.
  [x] When a prefab item is waiting (either new or edit), mark a waiting animation 
  [x] Change the "Gen3D" button to "Generate" button. Click it to show a fresh gen3d build panel. Add a place holder on the Prefabs panel immediately after the Build button is clicked. And replace the palce holder with the real item after gen3d save. Also mark a working or waiting animation on the thumbnail of place holder.
[x] On gen3d panel
  [x] Remove the "Clear Prompt" button.
  [x] Make the "Clear" button in the text box reacting  to both images and text: Either text and images existing can trigger the "Clear" button appearing. And click the "Clear" button to remove everything. 
  [x] Combine "Build" and "Continue" buttons: If it is a fresh build, then show the button as "Build", if it is a seeded build, then show the button as "Edit".
  [x] Be able to call gen3d by http without openning the real panel. And provide gen3d tasks list and status api
