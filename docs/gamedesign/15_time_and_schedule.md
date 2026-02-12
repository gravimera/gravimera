# Time, Schedules, and Living World Loops

Living worlds require a coherent notion of time. This document defines time semantics that work for both real-time play and deterministic simulation.

## Two Time Modes

### Real-time

- Simulation time advances continuously based on wall clock.
- Used for normal play and hosted always-on realms.

### Deterministic stepping

- Simulation time advances only when explicitly stepped (fixed dt).
- Used for automated tests, reproducible agent training, and debugging.

Realms choose whether deterministic stepping is allowed, and it is typically admin-only in hosting.

## World Clock

The realm has a **world clock** used for:

- day/night cycles,
- NPC schedules,
- timed story triggers,
- recurring world events.

The world clock is driven by simulation time, not wall clock. In deterministic mode it advances exactly by `dt * ticks`.

## NPC Schedules

An NPC schedule describes “where should this NPC be doing what at what times”.

Requirements:

- deterministic evaluation given the same starting state and tick stream,
- observable and editable (for creators/agents),
- safe to run at scale (bounded evaluation per tick).

Example schedule structure (conceptual):

- 06:00–09:00: go to “market” marker, role = vendor
- 09:00–17:00: stay near “shop” marker, role = shopkeeper
- 17:00–22:00: go to “home” marker, role = idle/social
- 22:00–06:00: go to “home” marker, role = sleep

Schedules can be implemented as:

- a built-in schedule brain, or
- a behavior graph with time conditions.

## Time-Driven Story

Story systems can trigger on time:

- “festival starts at 18:00”
- “guards change shift every 4 hours”
- “quest expires after 3 days”

These triggers must work in both real-time and deterministic stepping.

## Events

Time-related events include:

- `day_started`, `night_started` (optional)
- `schedule_transition` (NPC changed schedule block)
- `time_trigger_fired` (story time trigger)

These events are crucial for resident agents orchestrating world operations.

