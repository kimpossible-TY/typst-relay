## ADDED Requirements

### Requirement: Dependency pre-discovery does not execute document code

Tinymist SHALL collect statically known dependency targets without invoking runtime expression tracing during dependency pre-discovery.

#### Scenario: Import inside an uncalled function
- **WHEN** a semantic-token request analyzes `#let unused() = { import calc: max }`
- **THEN** dependency pre-discovery does not evaluate or lay out the document
- **AND** expression analysis may use its existing static module resolver

### Requirement: Unknown dependency targets remain conservative

Tinymist SHALL retain unresolved dependency information and allow expression analysis to record authoritative late edges.

#### Scenario: A dependency source cannot be resolved from a literal
- **WHEN** an import or include uses an alias or computed expression
- **THEN** pre-discovery marks the source as unresolved
- **AND** later resolution and cycle handling remain available

#### Scenario: A direct file dependency is known
- **WHEN** an import or include uses a resolvable string literal
- **THEN** the dependency is recorded relative to its source file

### Requirement: Semantic highlighting remains enabled

Tinymist SHALL preserve semantic-token responses for the regression fixture rather than avoiding work by returning empty data or disabling the feature.
