## Why

Repeated editing of a real book exposes independent costs beyond dependency pre-discovery: dot completion traces the document before consulting static types, cancelled requests retain obsolete work, and the existing cache policy retains ten unused intervals while repeated compilation grows the process footprint by several gigabytes. Fresh-process measurements and a package-isolated import A/B are recorded privately under `.local/investigation-20260922/`.

## What Changes

- Prefer sufficiently precise static dot-completion results; otherwise use the first evaluation observation when available, retaining full tracing for contextual values and styles.
- Handle cancellation of supported read-only LSP queries with exactly one response, protect reused request IDs, and admit semantic work through an asynchronous queue so cancelled queued work is dropped without entering analysis.
- Allow lifecycle, execute-command, and unknown extension requests to finish their state transitions even when the client requests cancellation.
- Reject queued queries whose document snapshot has been superseded, and keep CPU-heavy semantic work off runtime I/O workers.
- Answer source-only document highlights for opened files without waiting for semantic admission, preserving closed-file resolution and negotiated position encoding.
- Reserve one package-index prefetch worker per registry and buffer blocking parsing so completion neither queues redundant workers nor crosses the HTTP async bridge for every JSON byte.
- Treat late compile-status notifications after editor shutdown as closed-channel errors rather than aborting a background worker.
- Coalesce automatic global cache eviction, retain one unused interval, and measure comemo/source/VFS cleanup separately.
- Validate outputs, feature regressions, trace counts, cancellation, and repeated-edit memory/latency using the original packages.

## Capabilities

### New Capabilities
- `responsive-interactive-analysis`: Cancellable query scheduling and efficient static completion/cache reuse.

## Impact

Tinymist analysis/query completion, runtime request scheduling, sync-ls protocol handling, package-index reading, and project cache maintenance. Nested import resolution continues in `fix-field-access-import-items`. No dependency pins or book source text are changed. Document-specific layout non-convergence was validated separately through a local annotation-package A/B: 281-page output equivalence and removal of both warnings precede its backed-up local application. Server benchmarks retain the original packages. The final report records that companion patch and the workspace server-path correction separately from server behavior.
