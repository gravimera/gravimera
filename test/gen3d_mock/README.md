# Gen3D Mock Config (Isolated)

This folder provides an isolated config to run Gen3D with the mock backend and a slow
(simulated) build delay.

Usage:

- `GRAVIMERA_CONFIG=./test/gen3d_mock/config.toml cargo run`
- For a fully isolated cache/log dir:
  `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" GRAVIMERA_CONFIG=./test/gen3d_mock/config.toml cargo run`

Notes:

- `mock://gen3d` only works in debug/test builds.
- The mock backend does not accept reference images; use prompt-only tests.
