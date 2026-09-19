## Implementation

- [x] Create fork, local checkout, development plan, and change specification.
- [x] Remove runtime fallback from dependency pre-discovery.
- [x] Add focused analyzer regression coverage.
- [x] Add a portable LSP reproducer with response and analysis-count checks.

## Validation

- [x] Build an unmodified baseline and modified server using matching settings.
- [x] Run query tests and inspect snapshot differences (74 passed; no snapshot changes).
- [x] Run formatting and relevant clippy checks (query-only strict clippy passed; existing dependency lint failure recorded).
- [x] Run CLI/e2e checks (9 passed).
- [x] Compare minimal fixtures and the private book snapshot.
- [x] Record measured results, remaining runtime paths, and deployment status.
