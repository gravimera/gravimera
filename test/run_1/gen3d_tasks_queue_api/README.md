## Gen3D Tasks Queue API real test (mock://gen3d)

This folder contains a reproducible "real test" for the Gen3D tasks queue HTTP API:

- Starts Gravimera with `--automation` and `--automation-pause-on-start`.
- Uses `mock://gen3d` (no network / API keys required).
- Enqueues two Gen3D build tasks via `/v1/gen3d/tasks/enqueue`.
- Steps frames and polls `/v1/gen3d/tasks`, asserting:
  - FIFO execution
  - At most one task is `running` at a time
  - Both tasks reach `done` and produce `result_prefab_id_uuid`

### Run

From repo root:

```bash
python3 test/run_1/gen3d_tasks_queue_api/run.py
```

Artifacts:

- `tmp/` is used for per-run `GRAVIMERA_HOME` (logs, cache, realm data).
- Each run writes `tmp/run_*/gravimera_stdout.log` (kept on failure; auto-removed on success).
