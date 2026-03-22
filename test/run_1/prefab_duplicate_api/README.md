## Prefab duplicate API real test (`POST /v1/prefabs/duplicate`, mock://gen3d)

This folder contains a reproducible "real test" for duplicating realm prefab packages via the Automation HTTP API:

- Starts Gravimera with `--automation` and `--automation-pause-on-start`.
- Uses `mock://gen3d` (no network / API keys required).
- Generates a prefab via the Gen3D task queue (`/v1/gen3d/tasks/enqueue`).
- Duplicates it via `POST /v1/prefabs/duplicate`.
- Verifies:
  - new prefab id is different
  - new package exists on disk and has the expected files
  - duplicated prefab is present in `/v1/prefabs` and is spawnable via `/v1/spawn`

### Run

From repo root:

```bash
python3 test/run_1/prefab_duplicate_api/run.py
```

Artifacts:

- `tmp/` is used for per-run `GRAVIMERA_HOME` (logs, cache, realm data).
- Each run writes `tmp/run_*/gravimera_stdout.log` (kept on failure; auto-removed on success).

