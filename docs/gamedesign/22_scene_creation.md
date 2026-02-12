# Scene Creation (Realistic, Rich Worlds at Scale)

This document focuses on *scene creation* for Gravimera’s “living world / metaverse-like” goal: how humans and AI agents can create and edit scenes that feel realistic (e.g. an ancient town) while containing **lots of objects**.

The key challenge is scale: realism comes from many small details, but naively placing every detail as an independent object is too slow to author, too heavy to simulate, and too hard to keep stylistically coherent.

## Design Goals

1) **Fast authoring**: create a believable town in minutes (human) or a handful of API calls (agent).
2) **Stylistic coherence**: buildings, props, NPC looks, and terrain feel like one place.
3) **Controlled variation**: similar buildings without obvious clones; repeatable patterns with small differences.
4) **Editable after the fact**: agents and humans can iterate without rebuilding from scratch.
5) **Deterministic generation**: procedural layers compile into identical results given the same seed and inputs.
6) **Performance-aware**: the engine can render and simulate large towns through instancing, chunking, and budgets.

## Multi-Scale Authoring (Macro → Micro → Life)

Realism emerges when a scene is authored across multiple scales:

### 1) Macro: terrain + districts + road network

- base terrain shape (flat, valley, terrace, coastal)
- district regions (market, temple hill, docks, residential)
- roads/paths as splines and intersections
- major landmarks and sightlines

### 2) Meso: parcels + building massing + walls

- subdivide districts into parcels/lots
- building footprints and placement rules (setbacks, align to roads)
- walls/fences and gates
- plazas and courtyards

### 3) Micro: props + clutter + decals + vegetation

- street props: lamps, signs, carts, barrels
- clutter pockets: crates near docks, pottery near workshops
- vegetation: small trees, planters, weeds
- decals and surface variation (dirt near doorways, worn paths)

### 4) Life: NPC population + schedules + ambient behaviors

- NPC looks (clothing palette, hair styles, role props)
- schedules (market hours, patrol routes, home/work loops)
- ambient actions (talking, sweeping, carrying goods)

The engine should provide authoring tools and API primitives for each scale. Agents should not be forced to micromanage micro details; they should describe higher-level intent and let deterministic compilers fill in detail.

## Two-Layer Representation: “Procedural Layers” + “Baked Instances”

To handle large towns, a scene should support two complementary representations:

1) **Procedural layers** (authoring-time, compact):
   - road network polylines
   - district polygons
   - parcel subdivision parameters
   - scatter definitions (what to place, where, density, min distance)
   - building kit rules (types, variations, alignments)

2) **Baked instances** (runtime-time, concrete):
   - the compiled set of object instances actually present in the scene

### Compilation

Procedural layers compile into baked instances deterministically using:

- the scene seed (and optional per-layer seeds),
- stable ordering rules (no hash-order nondeterminism),
- deterministic id generation for compiled objects.

### Editing and Diff

Creators must be able to:

- edit a layer (e.g. move a road),
- recompile affected regions,
- preserve stable identities where possible (so story/NPC references don’t break),
- produce a diff-like change report (what objects changed).

This is essential for iterative AI authoring: “generate → validate → render → patch → reapply”.

## Style Packs: The Backbone of Realism

An “ancient town” feels real primarily because its assets share:

- consistent materials and colors,
- consistent proportions (door heights, roof angles),
- consistent decorative motifs,
- consistent NPC clothing and role props.

To achieve this at scale, scenes should reference a **style pack**:

### Style Pack Contents (Conceptual)

- material palette (stone/wood/plaster colors, metal accents)
- building kit:
  - wall segments, roof modules, windows/doors, beams
  - “facade grammar” parameters (window spacing, roof overhang)
- prop kit: carts, barrels, lanterns, signs, benches, plants
- NPC look kit:
  - clothing silhouettes + color palette
  - role accessories (vendor apron, guard helmet)
- lighting/atmosphere presets (warm dusk, foggy morning)

Style packs can be:

- human-authored,
- generated via Gen3D, then curated,
- imported as external asset packs.

The important property is that a scene can “lock” to a style pack so new additions remain coherent.

## Layout Tools for Towns (Generic, Not Hard-Coded)

The engine should provide general-purpose spatial tools that can create many kinds of scenes (towns, forests, ruins), not only “ancient towns”.

Useful primitives:

- **Spline placement**: place objects along a road/path spline with spacing and jitter.
- **Parcel subdivision**: split a polygon region into lots with target area ranges.
- **Edge alignment**: align building fronts to nearest road edge and keep set-backs.
- **Scatter with constraints**: place props with Poisson-disc spacing, slope limits, and “avoid zones”.
- **Cluster scatter**: place pockets of clutter around anchor points (docks, markets).
- **Marker system**: named points/areas used by portals, NPC schedules, and templates.

These tools are deterministic and parameterized; they avoid “magic heuristics” like “roads should always be X” by requiring explicit parameters.

## Building Variation Without Clones

Large towns need “same but different” buildings.

Recommended approach:

1) Build a small set of **building archetypes** (house, shop, shrine, inn).
2) Each archetype is a **parametric template**:
   - footprint size range
   - number of floors
   - roof type choices
   - window/door patterns
   - allowed materials from the style pack
3) A deterministic sampler chooses parameters per parcel based on:
   - parcel geometry
   - district type
   - a stable seed (scene seed + parcel id)

This yields variation while keeping coherence and determinism.

## Micro Detail Layers (Where Realism Comes From)

Props and decals sell realism. They should be authored as layers, not as hand-placed objects everywhere:

- “street dressing” layer: lamps/signs/benches placed along roads
- “economy traces” layer: crates near workshops, nets near docks
- “wear and tear” layer: decals for mud, worn stone, soot

These layers should be tunable per district and per road category.

## NPC Population: Looks, Roles, and Schedules

To make a town feel alive:

- Spawn NPC identities (stable identity ids) and bind them to:
  - a role (vendor/guard/citizen)
  - a home marker and a work marker
  - a schedule template
  - a look template from the style pack (clothing palette, accessories)

NPCs can be realized as:

- embedded brains (behavior graphs / schedule brain), and/or
- resident agents that orchestrate higher-level events.

The critical part for AI creation is **debuggability**:

- an agent can query “why is this NPC here?”
- the schedule and current brain node are inspectable.

## Validation: Make “Realism” Measurable

Agents need feedback loops. Provide validation outputs that are not subjective:

- **walkability**: can an NPC walk between major markers?
- **density**: objects per area; avoid over-cluttered extremes.
- **nav obstacles**: count and complexity estimates.
- **repetition**: detect identical building templates repeated too often in one region.
- **budgets**: estimated instance count, active brains, portals, event rates.

Validation can run as:

- a scene compiler report (per layer),
- an API endpoint (`scenes:validate`),
- and optionally a rendered snapshot for a vision model to critique (host policy).

The engine’s role is to provide deterministic measurements and tooling; any “aesthetic” judgement can be done externally by an agent using screenshots.

## Iterative AI Workflow (Practical)

A reliable creation loop for an agent:

1) Choose a style pack and a seed.
2) Submit a blueprint with:
   - district regions
   - road splines
   - initial archetype distribution
   - NPC population targets per district
3) Validate (budgets, walkability, repetition).
4) Apply.
5) Render a small set of fixed camera snapshots (top-down + street-level).
6) Critique externally and patch:
   - adjust a road spline
   - lower clutter density near intersections
   - increase roof variation
   - move landmarks to improve sightlines
7) Reapply with deterministic ids to preserve continuity.

This turns “AI freedom creation” into a stable engineering workflow rather than a one-shot generation.

## Example: “Ancient Town” as a Template

An ancient town template is just a composition of generic primitives:

- districts: market center, residential ring, temple hill, docks
- roads: main ring road + spokes + small alleys
- building archetypes: houses near residential; shops near market; shrine near temple hill
- props: lanterns along main roads, carts in market, crates at docks
- NPCs: vendor schedules in market hours; guards patrol ring road; citizens roam neighborhoods

Nothing here is hard-coded in the engine; it’s a realm-provided template + parameters + style pack.

