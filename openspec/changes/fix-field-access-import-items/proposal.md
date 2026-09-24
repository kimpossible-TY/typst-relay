## Why

Import statements such as `#import theoretic.presets.corners: theorem, lemma` are valid Typst, but static import analysis does not normalize intermediate module re-exports before selecting their fields. This also affects bare imports: Fletcher 0.5.8 uses `#import deps.cetz.draw`. Analysis of that source falls back to document tracing in both declaration and expression passes and still fails to bind the module. Repeated semantic-token requests consequently execute document layout even when every module in the import chain is statically known. This is the same resolver gap implicated by issue `#2410`.

## What Changes

- Resolve nested module re-exports during static analysis for bare imports, item imports, aliases, and wildcard imports.
- Populate imported bindings from those sources through the same import declaration path used for string and alias-based module imports.
- Resolve nested item paths segment by segment, instead of looking up their final name in the outer module.
- Keep unsupported non-module source expressions failing closed instead of manufacturing bindings from unresolved expressions.
- Reject recursive alias folding and preserve the existing import-component cycle handling.
- Add regression coverage for import bindings, definition navigation, nested item paths, unsupported sources, and zero document traces for statically known module chains.

## Capabilities

### New Capabilities
- `field-access-import-items`: Treat nested module-valued field-access expressions as valid static sources for bare imports and import item lists.

### Modified Capabilities
- None.

## Impact

- `crates/tinymist-query/src/syntax/expr.rs`
- Import-related fixtures under `crates/tinymist-query/src/fixtures/`
- No document, package-cache, dependency-version, or editor-configuration changes.
