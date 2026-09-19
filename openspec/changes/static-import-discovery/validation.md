# First implementation validation

Date: 2026-09-19. macOS arm64, 8 GiB RAM. Base source: `32f908199ee17ea295512bbc27166e890c438175` (0.15.8). These are local development results, not an installed extension release.

## Build and provenance

Both binaries use Rust 1.92.0, `cargo build --locked --release --bin tinymist`, `CARGO_BUILD_JOBS=2`, and `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`. Dependency pins and feature defaults are unchanged. Baseline was built in an unmodified detached worktree and copied before building the candidate. Because the target directory was shared, source modification time was refreshed to force Cargo to rebuild the changed query crate; the build log confirms recompilation from the candidate checkout. The binaries have distinct SHA-256 hashes recorded in ignored `.local/benchmarks/environment.json`.

## Initial measurements

No Rust build or other benchmark was run concurrently with these measurements. Existing user applications remained running. Each trial starts its own LSP server and terminates it afterward.

| Input | Baseline semantic-token response | Candidate response | Baseline / candidate completed dynamic traces |
|---|---:|---:|---:|
| Uncalled builtin import, costly layout | 5.550988 s | 0.003518 s | 1 / 0 |
| Control, same layout with local binding | 0.000744 s | 0.000743 s | 0 / 0 |
| Private book snapshot, main entry | Not completed within 25 s | 0.230 s | 5 / 0 |

The minimal fixture returns 63 tokens and the control 66 tokens. Candidate and baseline token arrays AND semantic-token legends match exactly for each fixture. The private book candidate returns 214 tokens; its baseline did not finish, so exact token equivalence for that document is not claimed. The baseline book trace count includes only completed calls observed before timeout, not all scheduled or in-flight work.

These are individual first-request measurements, not p95 values. `didOpen` can initiate normal diagnostic compilation; elapsed time measures end-to-end LSP response under that load. `analyze_expr` counts completed dynamic `typst::trace` analysis, not ordinary compilation. No preview or forwarded port was opened by this benchmark.

Portable reproduction:

```sh
python3 tests/perf/static-import-discovery/runner.py \
  --binary target/release/tinymist \
  --baseline .local/baseline/tinymist \
  --require-baseline-trace \
  --output-dir .local/benchmarks/minimal --timeout 30
```

Raw results, logs, private snapshot, and its file hash manifest remain ignored under `.local/`. The general two-line reproduction and runner are shareable under `tests/perf/static-import-discovery/`.

## Validation status

- Formatting and whitespace checks: passed.
- Python regression harness parser checks: 5 passed.
- Query Rust tests: 74 passed, including all five new regression tests and existing cyclic-package, semantic-token, hover, definition, and completion coverage. Existing snapshots required no changes.
- Query-only clippy with `--all-targets -- -D warnings`: passed.
- The initial clippy invocation also linted workspace dependencies and failed on pre-existing `clippy::undocumented_unsafe_blocks` at `crates/tinymist-std/src/fs/paths.rs:829` (macOS Time Machine exclusion). That file is unchanged from the base commit. It was not suppressed or edited as part of this patch; the relevant query crate was then checked with `--no-deps`.
- CLI/e2e: 9 passed, including compile/export smoke tests and LSP replay for editor initialization scenarios. The test executable was the candidate release copied into this checkout's ignored `editors/vscode/out/tinymist`; no installed editor extension was changed.

Validation commands:

```sh
CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  INSTA_UPDATE=no cargo test --locked -p tinymist-query

CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo clippy --locked --no-deps -p tinymist-query --all-targets -- -D warnings

CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  INSTA_UPDATE=no cargo test --locked -p tests --test tinymist-e2e-tests -- --test-threads=1
```

The benchmark book's complete file hash manifest remained unchanged after testing. A retained candidate binary and source patch are available in ignored `.local/candidate/`; build/check/test logs and binary hashes are under `.local/benchmarks/`.

## Remaining scope

Expression-stage dynamic fallbacks remain available. This patch specifically removes eager runtime tracing from dependency pre-discovery. Long-running memory stability, edit/cancel scheduling, remote preview reliability, and actual editor rollout remain later validation/development steps. The active editor has not been switched to this candidate.
