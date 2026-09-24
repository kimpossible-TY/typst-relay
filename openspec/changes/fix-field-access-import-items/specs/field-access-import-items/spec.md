## ADDED Requirements

### Requirement: Static import analysis resolves module-valued field-access sources
Tinymist SHALL treat a field-access expression that resolves through module re-exports to a module as a valid source for bare, named, aliased, and wildcard imports during static analysis, without executing the document to resolve that source.

#### Scenario: Bare nested module import is bound without tracing
- **WHEN** a user writes `#import deps.cetz.draw`, where each segment is a statically known imported module
- **THEN** tinymist binds `draw` to the selected module in both declaration and expression analysis
- **AND** semantic-token analysis does not invoke dynamic expression tracing for that import

#### Scenario: Bare module import preserves an exported alias name
- **WHEN** `deps.branch.leaf` refers to a module imported from a differently named file
- **THEN** `#import deps.branch.leaf` binds the module under `leaf`, matching Typst's bare import naming

#### Scenario: Simple item list from a field-access source is bound
- **WHEN** a user writes `#import theoretic.presets.corners: theorem, lemma` and `theoretic.presets.corners` resolves to a module
- **THEN** tinymist statically binds `theorem` and `lemma` to the corresponding exports from that module for editor-side semantic features

#### Scenario: Nested import item path is resolved from the selected module
- **WHEN** a user writes an import item path such as `#import pkg.section: nested.value` and `pkg.section` resolves to a module
- **THEN** tinymist resolves the item path against that module's export scope and binds the imported name to the targeted export
- **AND** an unrelated `value` export in `pkg.section` does not replace `nested.value`

#### Scenario: Unsupported non-module source stays unresolved
- **WHEN** a non-wildcard import item list uses a field-access source that does not resolve to a module
- **THEN** tinymist MUST NOT resolve the requested items to fabricated module exports
- **AND** unresolved import declarations may remain as syntax metadata for editor features

#### Scenario: Recursive aliases stay unresolved
- **WHEN** static module resolution encounters a reference or identifier cycle
- **THEN** folding terminates with an unresolved result without inventing a module

### Requirement: Supported field-access item imports do not degrade to empty static scope
Tinymist SHALL provide static import scope for supported module-valued field-access item imports instead of degrading them to an empty import scope that requires dynamic import analysis to recover editor behavior.

#### Scenario: Supported item import produces static scope immediately
- **WHEN** tinymist analyzes a supported import like `#import theoretic.presets.corners: theorem`
- **THEN** the imported item scope is available through static analysis for subsequent editor features in the file

#### Scenario: Existing wildcard field-access import behavior is preserved
- **WHEN** a user writes `#import lib.draw: *`
- **THEN** tinymist continues to resolve the wildcard import as before while adding support for non-wildcard item imports from module-valued field-access sources
