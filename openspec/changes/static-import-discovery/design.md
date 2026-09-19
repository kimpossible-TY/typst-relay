## Decision

Use `SharedContext::const_eval` directly in dependency discovery instead of `analyze_import`. A known string is resolved relative to the source file as before. Non-literal sites retain `has_unresolved`; expression analysis remains responsible for resolving aliases, builtin modules, and dynamic sources and recording late edges.

## Correctness boundaries

- Continue visiting imports and includes inside function bodies.
- Do not introduce a second lexical evaluator or mistake shadowed names for builtins.
- Keep unresolved literal paths conservative.
- Preserve `record_dependency`, component retraction, retry, and SCC merging behavior.
- Do not globally disable runtime analysis. Existing expression-stage runtime fallback is outside this first patch.

## Validation

Exercise relative literal imports/includes, unknown sources, unused builtin imports, aliases, shadowing, and existing cyclic/late-edge snapshots. For the unused builtin reproducer, assert nonempty semantic-token output and zero dynamic-expression calls; timing is supporting evidence only. Compare binaries using the same toolchain/profile, and preserve raw benchmark output outside git.
