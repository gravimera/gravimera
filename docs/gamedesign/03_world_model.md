# World Model: Realms, Scenes, Portals, Terrain

Gravimera’s world is a **realm** containing a graph of **scenes** connected by **portal gates**.

## Realms

A realm is the top-level unit of:

- persistence (save/load),
- packaging/sharing,
- permissions (who can edit vs. only play),
- simulation settings (real-time vs deterministic stepping).

Realms can be “single-author” (one creator agent) or “multi-author” (shared editing), but the permission model must be explicit either way.

## Scenes

A scene is a bounded play area with:

- terrain and obstacles,
- object instances (units/buildings/props/portals),
- scene-level metadata (biome, lighting preset, ambient audio, nav settings),
- local story variables and triggers (in addition to realm-global).

### Scene Graph (Multiple Scenes)

Scenes are connected by directed or bidirectional portals. A realm can represent:

- a hub world with multiple dungeons,
- a sequence of narrative chapters,
- a connected overworld split into regions.

## Portals (Travel Between Scenes)

A portal gate is an object instance with:

- a trigger volume (enter condition),
- a destination scene id,
- a destination spawn rule (exact transform, spawn marker id, or “nearest safe point”).

Travel can be:

- **single traveler** (one unit/player),
- **party travel** (selected group),
- **convoy travel** (units within radius).

The travel rule is part of the portal configuration.

### Portal Constraints (To Keep Travel Robust)

- Travel emits a realm event (`portal_entered`) with from/to scene ids and traveler ids.
- If the destination scene is not present, it can be created from a template (if permitted), otherwise travel fails with a clear error.
- Portals can be locked behind story variables (e.g. “requires_key = true”).

## Terrain (Built from Basic Objects)

Terrain is modeled as layers that all resolve to ordinary objects:

1) **Base surface layer**: a continuous ground surface (flat plane, heightmap, or voxel-like tile field).
2) **Obstacle layer**: rocks, walls, cliffs, platforms, bridges — all built from object instances with collision and “supports standing”.
3) **Decoration layer**: trees, props, particles, decals.

The key rule is: *a creator can build terrain out of the same object system as everything else*, and agents can author terrain via the same APIs.

### Navigation Implications

Scenes define navigation constraints:

- walkable surfaces (including atop objects that support standing),
- movement blockers (walls/fences),
- optional multi-level navigation (bridges, platforms).

This design favors **predictable authoring**: creators explicitly mark what blocks movement and what supports standing, rather than relying on automatic heuristics.

