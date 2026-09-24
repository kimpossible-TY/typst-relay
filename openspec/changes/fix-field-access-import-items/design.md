## Context

`ExprWorker::eval_expr` represents `deps.cetz.draw` as nested selections. A module's exported import binding is an `Expr::Ref`, whose root is the imported `Decl::Module`. Previously, folding the inner selection returned that reference unchanged. The outer selection only accepted a module declaration and therefore failed. `check_import` then traced the document, but its expression fallback only retained string paths and discarded a module result.

This occurs in both the declaration interface pass and the full expression pass. The earlier `static-import-discovery` change deliberately removed tracing only from dependency pre-discovery; it does not cover this expression-stage failure.

## Decisions

### Normalize selected references before further field selection

Fold the result of a static selection through existing reference and identifier resolution. Preserve selection-reference recording so editor navigation retains its source spans. A statically known module declaration then reaches `check_import_source_val` or `check_import_by_def` without document execution. Both passes share this implementation.

Use Typst's `ModuleImport::bare_name` for an implicit module binding. A re-exported module selected as `deps.branch.leaf` binds `leaf`, even when its defining file has a different stem.

Track expressions currently being folded in a path-local stack. A repeated expression on the same path returns unresolved instead of recursing. Pop expressions when leaving a path so legitimate repeated module selections remain possible. The stack avoids using expression values, which can transitively contain lazy caches, as hash-map keys. Module export lookup continues to use the existing component coordinator and late dependency recording; this change does not replace that cycle handling.

### Resolve import item paths from left to right

Start at the first exported item, then select each remaining segment against that item's normalized module or known value. This prevents `nested.marker` from incorrectly resolving to an unrelated `marker` in the outer module. Preserve unresolved selection expressions as syntax metadata; they must not become authoritative bindings to invented exports.

### Preserve dynamic fallback for genuinely unresolved sources

Do not disable runtime import analysis globally or alter Typst evaluation. Only statically resolvable module chains bypass it. Non-module and missing-field cases remain unresolved. The independent interactive-analysis change owns completion admission and cancellation.

## Validation

- Assert the target file of bare, named, aliased, wildcard, and nested-item imports using an in-memory, package-free module tree.
- Include a same-named outer export so nested-item tests detect accidental final-segment lookup.
- Assert zero increments in `analyze_expr` statistics for the importing file while producing nonempty semantic tokens.
- Test unsupported non-module sources and a synthetic recursive alias chain.
- Add a definition-navigation snapshot for a nested field-access item import.
- Run existing query snapshots, especially cyclic/late-edge import fixtures, and inspect any changes before accepting them.
- Re-run the private repeated-edit benchmark with the rebuilt server and the original, unmodified Fletcher package to confirm the measured trigger no longer traces.
