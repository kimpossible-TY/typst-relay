#import "mod.typ": *
#show: book-page.with(title: "Typst Relay")

*A responsive language server and live preview for Typst.*

Typst Relay brings semantic highlighting, completion, navigation, formatting, and live preview to Typst projects. Its development focuses on keeping editing responsive as documents, diagrams, and package dependencies grow.

Forked from #link("https://github.com/Myriad-Dreamin/tinymist")[Tinymist] by Myriad-Dreamin and its contributors.

#link("#get-started")[Get started] · #link("#performance")[Performance] · #link("#development")[Development] · #link("https://github.com/kimpossible-TY/tinymist/issues")[Issues]

= What Typst Relay does

- *Understand code without running the whole document.* Static dependency and module resolution avoid unnecessary document execution during semantic analysis.
- *Keep editing requests moving.* Supported read-only requests can be cancelled, and obsolete queued work is discarded when the document changes.
- *Control retained work.* Coalesced cache cleanup limits retained generations, while source-only highlighting stays responsive during heavier analysis.
- *Use the tools you already have.* Typst Relay runs behind the existing Tinymist editor extension, with Typst preview, navigation, formatting, and completion in the same workspace.

Typst Relay is under active development. Context-dependent expressions can still require document layout, and running computations finish before releasing shared analysis resources.

= Get started

== Build

Install Rust through rustup and the native build tools for your operating system. The repository pins the Rust toolchain.

```bash
git clone https://github.com/kimpossible-TY/tinymist.git
cd tinymist
cargo build --locked --release --bin tinymist
./target/release/tinymist probe
```

On Windows, the executable is `target/release/tinymist.exe`. On a memory-constrained macOS/Linux host, prefix the build command with `CARGO_BUILD_JOBS=2`.

== Connect VS Code

Install and enable the existing Tinymist extension. In your VS Code settings, select the built executable:

```json
{
  "tinymist.serverPath": "/absolute/path/to/tinymist/target/release/tinymist",
  "tinymist.semanticTokens": "enable"
}
```

Use an absolute path. A Windows example is `C:/dev/tinymist/target/release/tinymist.exe`.

For SSH, Tunnel, WSL, or Dev Container sessions, build on the machine running the remote extension host and put that machine's executable path in *Remote Settings*. Workspace settings take precedence, so remove any stale workspace override.

Save your work and run *Developer: Reload Window*. Open a Typst file and start preview with the extension's usual preview command. Typst Relay supplies the server; a separate extension or manually started LSP process is not required.

The executable and configuration identifiers remain `tinymist` for editor compatibility. Initial editor deployment was verified with the Tinymist 0.15.8 extension on macOS arm64. Check compatibility when changing extension versions.

== Update

```bash
git switch main
git pull --ff-only origin main
cargo build --locked --release --bin tinymist
```

Reload the VS Code window after rebuilding. To use the extension's bundled server again, remove `tinymist.serverPath` from the settings scopes that define it and reload.

= Performance

The latest repeated-edit investigation used an 8 GiB macOS arm64 machine and a 281-page Typst book with its original packages. The following medians compare only the same completed editing rounds in the preserved earlier server build and the optimized build:

#table(
  columns: 4,
  table.header([Operation], [Shared rounds], [Earlier build], [Optimized build]),
  [Semantic tokens], [2], [7.994 s], [0.394 s],
  [Compile after edit], [3], [4.385 s], [4.408 s],
  [Contextual completion], [2], [4.558 s], [4.870 s],
)

The optimized build completed all 12 edits in each of four scenarios, with peak process footprints of 3.674–4.734 GiB. The earlier build reached a 5 GiB diagnostic guard after 1–3 completed edits. Semantic-token full-document traces fell from two per edit to zero, and all eight comparable successful query responses matched.

These are workload-specific observations from sequential runs on a shared machine. Ordinary compilation was approximately unchanged, and contextual completion was slightly slower. Twelve edits do not establish indefinite memory stability. Remote preview transmission and client rendering were not measured.

The server-only output comparison preserved all 281 pages: extracted text, word coordinates to 0.001 pt, and rendered pixels at 96 ppi matched. See the #link("docs/tinymist/dev/interactive-analysis-performance.typ")[investigation report source] for methods, validation, and remaining limits. Measurements describe the September 23 candidate; subsequent integration with main has separate regression checks.

= Development

#link("https://github.com/kimpossible-TY/tinymist")[kimpossible-TY/tinymist] is the development repository, and `main` is the integration branch. The earlier separate `tinymist-flow` repository is retained as historical storage.

- #link("docs/dev-guide.md")[Developer guide] — toolchain, crates, and editor tooling.
- #link("DEVELOPMENT_PLAN.md")[Development plan] — current priorities and investigation history.
- #link("openspec/changes/optimize-interactive-analysis/tasks.md")[Interactive analysis work] — implemented behavior and validation.
- #link("tests/perf/static-import-discovery/runner.py")[LSP regression harness] — a reproducible static-import workload.
- #link("https://myriad-dreamin.github.io/tinymist/")[Tinymist documentation] — shared configuration and editor features.

Set `upstream` to `https://github.com/Myriad-Dreamin/tinymist.git` when syncing shared changes. Contributions intended for the original project should start on a separate branch based on `upstream/main`.

README is generated from `docs/tinymist/typst-relay.typ`. Edit that source and regenerate with `node scripts/link-docs.mjs --readme-only` after building the `typlite` binary. See #link("AGENTS.md")[AGENTS.md] for repository conventions.

= License

Distributed under #link("LICENSE")[Apache License 2.0]. Existing copyright and third-party notices are retained.
