# Run 1

`test/run_1/scripts/run_real_gen3d_http_test.sh` performs a rendered Automation HTTP API Gen3D run against a local Gravimera process.

`test/run_1/gen3d_expression_edit_regression/run.py` performs a rendered real-provider seeded-edit regression against an isolated copied prefab package and fails if unrelated motion channels are rewritten during a named-expression edit.

It writes these generated artifacts under `test/run_1/`:

- `config.toml`
- `home/`
- `logs/`
- `responses/`

The script reads the real Gen3D provider settings from `~/.gravimera/config.toml`, writes an isolated automation-enabled config here, starts Gravimera with `GRAVIMERA_HOME=test/run_1/home`, drives the HTTP API, and leaves the resulting artifacts in place for inspection.
