## 1. Static completion
- [x] 1.1 Implement precise static-first dot completion and evaluation-first runtime completion with full-trace/style fallback.
- [x] 1.2 Add and review completion/trace regressions, including first-value/style equivalence.

## 2. Request scheduling
- [x] 2.1 Implement read-only LSP cancellation, exactly-one terminal response, reused-ID protection, and stateful-request completion.
- [x] 2.2 Add cancellable semantic admission, stale-snapshot checks, and native blocking execution.
- [x] 2.3 Add regressions for cancellation races, queued work suppression, and permit lifetime.
- [x] 2.4 Keep source-only highlights for opened documents responsive outside semantic admission while preserving closed-file fallback and position encoding.
- [x] 2.5 Reserve one package-index prefetch worker per registry, buffer response reads, and validate concurrency, cached results, metadata, malformed JSON, and I/O failure behavior; keep unrelated workspace tests independent of network prefetch.
- [x] 2.6 Tolerate late compile-status notifications after editor shutdown and add a worker regression.

## 3. Cache maintenance
- [x] 3.1 Implement coalesced single-flight automatic eviction and recent-interval retention.
- [x] 3.2 Split eviction timing and add serialization/edit-revert output regressions.

## 4. Validation
- [x] 4.1 Run affected crate tests, review snapshots, formatting, and strict clippy.
- [x] 4.2 Validate native/web feature combinations and CLI/e2e coverage.
- [x] 4.3 Build a separate candidate and compare repeated editing with original packages, including correctness and memory/latency.
- [x] 4.4 Record measured results and remaining document-specific issues in source documentation.

## Implementation evidence

These checks record implementation and reviewed test coverage; test execution, feature validation, and performance acceptance remain tracked separately in section 4.

- Tasks 1.1–1.2: `crates/tinymist-query/src/analysis/completion/field_access.rs`, `crates/tinymist-analysis/src/track_values.rs`, and the `analyze_expr_first` query wrapper. Fixtures cover module exports, builtin aliases/shadowing, math interpolation, arrays, mutated dictionary keys, contextual callbacks, and first-observation ordering.
- Task 2.1: `crates/sync-lsp/src/server.rs` owns request identities, abort registrations, and the read-only cancellation policy; `server/lsp_srv.rs` dispatches `$/cancelRequest`. Regressions include `cancelled_future_cannot_take_a_reused_ids_registration`, `cancelled_ready_response_cannot_complete_a_reused_id`, and `stateful_requests_finish_before_responding_despite_cancellation`.
- Tasks 2.2–2.3: `crates/tinymist/src/query_queue.rs` and `lsp/query.rs`; `changed_snapshot_never_runs`, `cancelled_waiter_never_runs`, and `cancelled_running_work_keeps_its_permit` cover the queue's scheduling boundaries.
- Tasks 3.1–3.2: `crates/tinymist-project/src/compiler/eviction.rs` implements automatic sweep ownership, coalescing, and `MAX_UNUSED_AGE = 1`. Its concurrent-burst regression checks one follow-up sweep; `compiler.rs::recent_cache_retention_preserves_edit_and_revert_output` checks imported-dimension edits and reverts through cache expiration. `compiler.rs` and the helper report separate cleanup durations.

## Validation evidence

- Query tests: 89 passed. Final runtime tests: 59 passed, 3 existing ignored. Project tests: 19 passed, 5 existing ignored. Cancellation tests: 12 passed. Package parser/prefetch tests: 4 passed. Reviewed snapshots, formatting, and warning-denying clippy pass.
- All ten native/web feature checks and nine final CLI/editor integration tests pass against candidate `bd837663`.
- Four original-package replay modes each complete 12 edits. A fresh original binary reaches the 5 GiB diagnostic guard after 3/2/2/1 completed edits. Eight comparable successful query responses match; semantic-token full traces fall from two per edit to zero. Matched compile latency is unchanged, while contextual completion is 6.8% slower over its two comparable edits. This limitation and guard censoring are reported explicitly.
- Server-only and separate package-only full-book comparisons preserve all 281 rasterized pages, text, and word geometry. The package-only patch removes both convergence warnings; backed-up source/installed application is verified through the actual package directory. Workspace server-path override removal selects the tested release on the next editor reload.
- Canonical report: `docs/tinymist/dev/interactive-analysis-performance.typ`. Private raw measurements, binary/source hashes, and application backups remain under `.local/optimization-20260923/` and `.local/investigation-20260923/`.
