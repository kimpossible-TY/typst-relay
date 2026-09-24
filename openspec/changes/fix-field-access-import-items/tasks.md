## 1. Resolve module-valued field-access sources

- [x] 1.1 Normalize intermediate module re-export references so `check_import` recognizes nested field-access sources for bare, named, aliased, and wildcard imports.
- [x] 1.2 Resolve nested import item paths against each selected module, while keeping unsupported sources unresolved.
- [x] 1.3 Add path-local cycle detection to static folding and preserve existing component-based export lookup.

## 2. Add regression coverage

- [x] 2.1 Add scope and zero-trace regressions for bare, named, aliased, wildcard, and nested item imports through module re-exports.
- [x] 2.2 Add a goto-definition fixture showing a name imported from a nested field-access source resolves to the exported definition.
- [x] 2.3 Add regressions for an unrelated same-named outer export, an unsupported non-module source, and recursive alias folding.

## 3. Validate the change

- [x] 3.1 Run focused `tinymist-query` import-analysis and editor-feature tests that exercise the new field-access item import coverage and review the resulting snapshots.
- [x] 3.2 Run query regression tests including existing cyclic imports, formatting, and warning-denying query lint checks.
- [x] 3.3 Re-run the repeated-edit benchmark against the original Fletcher package and record dynamic trace counts and latency.
