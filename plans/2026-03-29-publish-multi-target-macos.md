# Multi-target publish.py packaging

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` in the repository root.

## Purpose / Big Picture

After this change, a contributor can run one `tools/publish.py` command on macOS and produce both Apple Silicon and Intel macOS distribution packages without one package overwriting the other. The same script should also package based on the requested Rust target triple rather than blindly assuming the host operating system, so explicit targets produce correctly named artifacts and executable paths.

How a human verifies it works:

- On a macOS machine, run `python3 tools/publish.py --target aarch64-apple-darwin --target x86_64-apple-darwin`.
- Observe two separate macOS zip artifacts under `dist/macos/`, one per target triple, with matching app bundle directories.
- Start the game normally with the required rendered smoke test and confirm the app still launches without crashing.

## Progress

- [x] (2026-03-29 11:18 CST) Review the current `tools/publish.py` flow and confirm it derives platform-specific behavior from `sys.platform`, which breaks explicit cross-target packaging.
- [x] (2026-03-29 11:24 CST) Draft this ExecPlan and settle on a target-driven multi-target packaging design with repeatable `--target`.
- [x] (2026-03-29 12:00 CST) Update `tools/publish.py` to accept multiple targets, derive packaging/executable naming from the target triple, and generate non-conflicting macOS artifact names.
- [x] (2026-03-29 12:01 CST) Update publishing documentation with the new multi-target workflow and limitations.
- [x] (2026-03-29 12:29 CST) Validate the new publish flow with both macOS targets, run the required rendered smoke test, and prepare the commit.

## Surprises & Discoveries

- Observation: `tools/publish.py --target <triple>` was only half-implemented before this change. The build command respected `--target`, but binary naming and package format still came from the host OS.
  Evidence: `_release_bin_path()` used `sys.platform` to choose `gravimera.exe` vs `gravimera`, and `main()` always selected `_package_macos` on macOS hosts.

- Observation: The first full release build for each macOS target spent most of its wall time in the final `rustc` link step for `src/main.rs`, not in Python packaging.
  Evidence: `cargo build --release --bin gravimera --target aarch64-apple-darwin` finished in 10m 40s, and `cargo build --release --bin gravimera --target x86_64-apple-darwin` finished in 10m 23s before the zip files were written.

## Decision Log

- Decision: Use repeatable `--target` flags instead of introducing a separate macOS-only option.
  Rationale: The user-visible problem is “one command, many targets.” Repeatable `--target` is simple, matches Cargo terminology, and extends cleanly to any future target triples the local toolchain can build.
  Date/Author: 2026-03-29 / Codex

- Decision: When packaging an explicit target, include the target triple in artifact names and macOS app bundle names.
  Rationale: This avoids overwrite bugs and makes artifacts self-describing when a single run emits multiple packages.
  Date/Author: 2026-03-29 / Codex

- Decision: Validate requested explicit targets against `rustup target list --installed` and fail with `rustup target add ...` guidance when a target is missing.
  Rationale: The packaging tool cannot auto-provision a Rust standard library or platform toolchain safely, but it can fail fast with an actionable fix instead of surfacing a less clear downstream Cargo error.
  Date/Author: 2026-03-29 / Codex

## Outcomes & Retrospective

- (2026-03-29 12:29 CST) `tools/publish.py` now treats explicit targets as first-class build specs. One command on macOS can build and package both `aarch64-apple-darwin` and `x86_64-apple-darwin`, producing `Gravimera-<target>.app` plus `gravimera-0.1.0-macos-<target>.zip` without overwrite. The docs now explain the repeatable `--target` workflow and the `rustup target add ...` prerequisite. The required rendered smoke test also passed, so the packaging change did not regress startup.

## Context and Orientation

The relevant packaging code lives in `tools/publish.py`. It currently:

- reads the app version from `Cargo.toml`,
- optionally runs `cargo build --release --bin gravimera [--target ...]`,
- copies the built binary and assets into a distribution layout,
- writes output under `dist/<platform>/`.

The current limitation is that `tools/publish.py` treats the host system as the package platform. That means a macOS host always emits a macOS app bundle, even if the user requested a different target triple, and it names the release binary based on the host platform instead of the target triple. It also uses the same `Gravimera.app` and `gravimera-<version>-macos.zip` names for every macOS build, so a second target overwrites the first one.

## Plan of Work

In `tools/publish.py`, introduce a small target-description layer that maps either the implicit host build or an explicit Rust target triple to:

- package platform (`macos`, `linux`, `windows`),
- built executable filename (`gravimera` or `gravimera.exe`),
- output artifact suffixing rules.

Change argument parsing so `--target` can be repeated. The script will then loop over the requested targets, build each one, locate the correct release binary under `target/<triple>/release/`, and package it using the platform implied by that target triple. For explicit targets, package names must include the target triple. For the existing host-default invocation with no explicit target, keep the current simpler names.

Update `docs/publishing.md` so it documents both the original single-platform command and the new multi-target macOS example, including the requirement that the relevant Rust targets must be installed first.

## Concrete Steps

Work from the repository root:

    rustup target add x86_64-apple-darwin
    python3 tools/publish.py --help
    python3 tools/publish.py --target aarch64-apple-darwin --target x86_64-apple-darwin
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Observed outcomes:

- `--help` shows that `--target` can be repeated.
- The dual-target publish command writes distinct files such as `gravimera-0.1.0-macos-aarch64-apple-darwin.zip` and `gravimera-0.1.0-macos-x86_64-apple-darwin.zip`.
- The rendered smoke test exits successfully after 2 seconds.

## Validation and Acceptance

Acceptance is met when:

- One `tools/publish.py` invocation can package two macOS targets without overwrite.
- Explicit targets are packaged according to the target triple rather than the host OS.
- `docs/publishing.md` shows the new workflow and target-install prerequisite.
- The required rendered smoke test passes.

## Idempotence and Recovery

Running the publish command multiple times is safe. The script may overwrite same-named output artifacts for the same target triple, which is expected for packaging. If a requested target is missing from the local Rust installation, the recovery path is to install it with `rustup target add <triple>` and re-run the same command.

## Artifacts and Notes

Validation evidence:

    $ python3 tools/publish.py --no-build --target aarch64-apple-darwin --target x86_64-apple-darwin
    Wrote /Users/flow/workspace/github/gravimera/dist/macos/gravimera-0.1.0-macos-aarch64-apple-darwin.zip
    Wrote /Users/flow/workspace/github/gravimera/dist/macos/gravimera-0.1.0-macos-x86_64-apple-darwin.zip

    $ find dist/macos -maxdepth 1 -mindepth 1 -print | sort
    dist/macos/Gravimera-aarch64-apple-darwin.app
    dist/macos/Gravimera-x86_64-apple-darwin.app
    dist/macos/gravimera-0.1.0-macos-aarch64-apple-darwin.zip
    dist/macos/gravimera-0.1.0-macos-x86_64-apple-darwin.zip

    $ tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2
        Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.29s
         Running `target/debug/gravimera --rendered-seconds 2`
    ... Creating new window Gravimera ...

## Interfaces and Dependencies

The implementation stays within the existing Python standard-library-only script. No new runtime dependency is required. The command-line interface at the end of the change should still support:

    python3 tools/publish.py

and additionally:

    python3 tools/publish.py --target aarch64-apple-darwin --target x86_64-apple-darwin

The packaging helpers in `tools/publish.py` must accept enough metadata to name app bundles and archives without relying on `sys.platform`.

---

Plan update (2026-03-29 12:29 CST): Marked implementation and validation complete after running the dual-target macOS publish flow, confirming the target-suffixed app bundles and zip files, and recording the required rendered smoke test result.
