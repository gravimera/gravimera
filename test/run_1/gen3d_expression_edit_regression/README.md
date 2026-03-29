# Gen3D Expression Edit Regression

Real-provider HTTP regression for seeded expression edits.

What it checks:

- starts Gravimera in rendered mode with Automation HTTP API enabled
- uses an isolated `GRAVIMERA_HOME`
- copies one existing Gen3D prefab package from the real home into that isolated home
- seeds an edit on that prefab and asks for one new named expression channel
- saves the overwrite edit
- fails if any `llm_generate_motions_v1` call in the run requested channels other than the new named channel
- verifies the saved prefab package contains the new named channel
- records any unrelated edit-bundle channel rewrites for inspection, but does not fail on them because preserve-mode basis rebasing can legitimately rewrite serialized slots without changing the visual motion

Usage:

```bash
SOURCE_PREFAB_ID=<prefab-id> python3 test/run_1/gen3d_expression_edit_regression/run.py
```

Optional env vars:

- `TARGET_CHANNEL` default: `shy_smile`
- `PROMPT` default: `Add a new expression animation channel named <TARGET_CHANNEL>. Keep the existing geometry, look, and existing motion channels unchanged.`
