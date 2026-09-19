## Why

Dependency discovery calls the runtime import tracer for non-literal sources, including imports inside uncalled functions. A semantic-token request can consequently perform full document layout before expression analysis resolves the same import statically. The package-free `import calc: max` reproducer regresses between 0.15.2 and 0.15.4 and remains slow in 0.15.8.

## What Changes

- Limit dependency pre-discovery to existing constant evaluation and path resolution.
- Preserve unresolved edges for later authoritative expression analysis and component merging.
- Add analyzer coverage and a portable LSP performance regression harness.

## Capabilities

### New Capabilities
- `static-import-discovery`: dependency pre-discovery does not execute the document.

## Impact

`crates/tinymist-query/src/analysis/pdg.rs` and regression tests. No compiler, editor configuration, dependency pin, or document-language changes. This is distinct from the active `fix-field-access-import-items` proposal, which changes item binding resolution rather than pre-discovery.
