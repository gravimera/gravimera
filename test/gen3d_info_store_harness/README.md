# Gen3D Info Store Harness (Fixture)

This folder contains a tiny, checked-in **Gen3D run-dir fixture** with an `info_store_v1/` JSONL store.

It is used as a regression harness to ensure:

- paging cursors work (no “latest is missing due to truncation” failures),
- KV selectors work deterministically,
- events/blobs can be inspected without exposing filesystem paths to the agent.

The fixture run dir lives under:

- `test/gen3d_info_store_harness/example_run/`

To run the harness test:

- `cargo test -q gen3d_info_store_fixture_harness`

