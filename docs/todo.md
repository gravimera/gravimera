[x] When double clicking an object on scene
  [x] Don't show the Preview panel anymore.
  [x] Select the prefabs and scroll the Prefabs panel to the selected item.
[x] ESC key also quit Prefabs panel.
[x] Rearrage the view of gen3d:
  [x] Remove The "Preview" mark.
  [x] Remove the "Collision" switch
  [x] Move the Status panel to left side. And move the running status to the right side.
  [x] Remove the "Realm" button. Add the "Exit" button on the top right side. And ESC key also triggers exit.
[x] In gen3d, if user provided images:
  [x] We also provide the images to LLM component generations, to make it more accurate.
  [x] To avoid huge images and blow up the LLM context size: 1. We only choose at most 2 images for a LLM component generation call. 2. If an image is too large, create a new image with lower resolution and use it for LLM component generations.
