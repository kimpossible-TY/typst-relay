#import "mod.typ": *
#show: book-page.with(title: "Tinymist Flow")

*Keep the colors. Keep writing.*

A performance-focused server fork of #link("https://github.com/Myriad-Dreamin/tinymist")[Tinymist] for Typst documents with expensive analysis paths. Built to keep semantic highlighting useful without making dependency discovery execute the document.

*Status:* experimental · based on Tinymist 0.15.8 · Apache-2.0

#link("#get-started")[Get started] · #link("#measured-results")[Measured results] · #link("DEVELOPMENT_PLAN.md")[Development plan] · #link("https://myriad-dreamin.github.io/tinymist/")[Upstream documentation]

= Why Flow?

Typing should not trigger document layout just to discover an import. In the investigated regression, an import inside an uncalled function could send semantic-token analysis through runtime tracing. The command-line compiler remained fast on the user's document while editor requests stalled.

Flow starts with a narrow fix: discover literal dependencies statically, leave unresolved expressions for later analysis, and preserve semantic tokens. It keeps the existing Tinymist extension and replaces the server executable behind it.

```text
VS Code + existing Tinymist extension
                  |
         tinymist.serverPath
                  |
         Tinymist Flow server
                  |
    Typst analysis and preview
```

= What changed

- *Static dependency discovery.* Import/include discovery no longer falls back to runtime tracing for non-literal sources.
- *Semantic colors stay on.* The regression fixture returns exactly the same token array and legend as upstream.
- *Reproducible checks.* A standalone LSP harness compares the patched server with an unmodified baseline.

Imports inside uncalled functions are still inspected. Other dynamic-expression fallback paths remain; this patch does not promise zero tracing for every document or solve every preview delay.

= Measured results

Single fresh-server semantic-token requests, measured on an 8 GiB macOS arm64 machine with matching Rust toolchains and release build settings:

#table(
  columns: 3,
  [*Input*], [*Upstream 0.15.8*], [*Flow, first patch*],
  [Unused-import reproducer], [5.551 s], [3.518 ms],
  [Control without the import], [0.744 ms], [0.743 ms],
  [Private document entry], [Timed out after 25 s], [230 ms],
)

The reproducer deliberately amplifies unnecessary tracing with expensive layout. These are individual measurements, not typical latency or a general speedup claim. Token arrays and legends match for the reproducer and control; the timed-out private baseline cannot establish token equivalence. The private document is not distributed.

The first patch passed 74 query tests, 9 CLI/LSP end-to-end tests, and 5 harness tests. Long editing sessions, memory stability, and remote preview delivery still need validation. See the #link("openspec/changes/static-import-discovery/validation.md")[measurement and validation record] for methodology and limits.

= Get started

Use the existing Tinymist VS Code extension. No separate extension or VSIX is required. The initial deployment used extension version 0.15.8; compatibility with later versions has not been established.

Build on the machine where the extension host runs. Install Rust through rustup; this checkout pins its toolchain.

```bash
git clone https://github.com/kimpossible-TY/tinymist-flow.git
cd tinymist-flow
cargo build --locked --release --bin tinymist
```

For a machine with limited memory, use `CARGO_BUILD_JOBS=2` before the build command. The executable remains named `tinymist`.

Set these values in the appropriate VS Code workspace or remote settings, using your actual absolute path:

```json
{
  "tinymist.serverPath": "/absolute/path/to/tinymist-flow/target/release/tinymist",
  "tinymist.semanticTokens": "enable"
}
```

Run *Developer: Reload Window* so the extension picks up the executable path, then reopen the preview. For SSH or tunnel sessions, the path must exist on the extension-host machine. To return to the bundled server, remove the `tinymist.serverPath` override and reload the window.

= Reproduce the regression

Requires Python 3.9 or newer. Build an unmodified Tinymist 0.15.8 baseline separately with matching compiler and build settings, then run:

```bash
python3 tests/perf/static-import-discovery/runner.py \
  --binary target/release/tinymist \
  --baseline /absolute/path/to/baseline/tinymist \
  --require-baseline-trace
```

The harness checks nonempty semantic-token output, exact token/legend equality, and analysis trace counts. Generated results stay outside version control. See #link("tests/perf/static-import-discovery/runner.py")[the harness] and #link("openspec/changes/static-import-discovery/design.md")[the patch design].

= Next steps

- Validate extended editing sessions with semantic highlighting enabled.
- Measure memory use and preview responsiveness in remote sessions.
- Investigate remaining dynamic analysis paths with reproducible fixtures.
- Keep fixes small enough to review and contribute upstream.

See #link("DEVELOPMENT_PLAN.md")[the development plan] and #link("PUBLIC_READINESS.md")[the publication review]. Open issues about this fork in #link("https://github.com/kimpossible-TY/tinymist-flow/issues")[this repository].

= Development and documentation

The runtime patch is in `crates/tinymist-query/src/analysis/pdg.rs`. Existing crate names, extension identifiers, and upstream version metadata are retained. The first preserved patch is tagged `fork-v0.15.8-p1`.

This README is generated from `docs/tinymist/flow.typ`. To regenerate only this page:

```bash
cargo build --locked --bin typlite
node scripts/link-docs.mjs --readme-only
```

The upstream documentation source remains available in `docs/tinymist/introduction.typ`. See #link("docs/dev-guide.md")[the developer guide] for the broader repository workflow.

= Attribution and license

Tinymist Flow is an independent derivative of #link("https://github.com/Myriad-Dreamin/tinymist")[Tinymist], created by Myriad-Dreamin and its contributors. It is not an official upstream release. The language server, preview system, and editor integrations originate from that project; this fork currently adds a focused dependency-discovery fix and its regression coverage.

Distributed under #link("LICENSE")[Apache License 2.0]. Existing copyright, attribution, and third-party notices are retained. Upstream documentation describes upstream capabilities; it is not a claim that upstream CI validates this fork.
