## Gen3D Tasks Queue API real test (seeded edit/fork + motion)

This folder contains an end-to-end "real test" for the Gen3D task queue API using `mock://gen3d`
(no network / API keys required).

Coverage:

- `POST /v1/gen3d/tasks/enqueue` with:
  - `kind=build` (snake + warcar)
  - `kind=edit_from_prefab` (overwrite-save)
  - `kind=fork_from_prefab` (new prefab id)
- FIFO queue behavior + "only one running at a time"
- Confirms tasks run while staying in Build Realm (no `build_preview` scene switch)
- Verifies motion authoring: the generated snake prefab package contains at least one animation slot with `channel="move"`

### Run

From repo root:

```bash
python3 test/run_1/gen3d_tasks_queue_seeded_api/run.py
```

Artifacts:

- `tmp/` is used for per-run `GRAVIMERA_HOME` (logs, cache, realm data).
- Each run writes `tmp/run_*/gravimera_stdout.log` (kept on failure; auto-removed on success).

