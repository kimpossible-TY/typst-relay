## ADDED Requirements

### Requirement: Precise static completion avoids execution
Tinymist SHALL use proven static dot-completion information without tracing the document and SHALL preserve dynamic fallback for incomplete or style-dependent information.

#### Scenario: Known module fields
- **WHEN** a dot target has a statically resolved module surface
- **THEN** completion returns its fields without a runtime trace

#### Scenario: Postfix completion on a known module
- **WHEN** a statically resolved imported module is a valid postfix target
- **THEN** completion preserves the exported fields and their signatures
- **AND** valid postfix snippets are available even when runtime evaluation does not observe the target

#### Scenario: Runtime-only target
- **WHEN** static information cannot supply the target's completion surface
- **THEN** existing runtime completion remains available

### Requirement: Runtime completion preserves its first observation
For completion consumers that select the first traced value, Tinymist SHALL use an evaluation observation when available and SHALL retain full tracing when evaluation does not observe the target.

#### Scenario: Runtime dictionary mutation
- **WHEN** evaluating the document observes a dictionary with dynamically inserted keys
- **THEN** completion includes those keys without laying out the document
- **AND** the selected value matches the first observation from a full trace

#### Scenario: Contextual value or styles
- **WHEN** the target is only observed during layout
- **THEN** completion uses full tracing and preserves its value and active styles

### Requirement: Obsolete queued analysis does not execute
The server SHALL respond exactly once to a cancelled supported read-only query and SHALL discard cancelled or superseded queued semantic analysis before it begins. Cancellation is supported for `textDocument/*`, `workspace/symbol`, and `workspace/willRenameFiles`.

#### Scenario: Cancellation while queued
- **WHEN** a client cancels a document query waiting for semantic admission
- **THEN** it receives RequestCancelled and its analysis closure is not executed

#### Scenario: Document changed before admission
- **WHEN** a pending semantic query refers to an older synchronized document revision
- **THEN** it returns ContentModified without analyzing that obsolete snapshot

#### Scenario: Pin mode changes without changing the primary entry
- **WHEN** user or preview pinning changes whether queries use the primary project or their own document world
- **THEN** queued queries captured under the previous pin mode return ContentModified even if the primary entry is unchanged

#### Scenario: Effective pin mode is unchanged
- **WHEN** pin flags change but another active pin keeps queries in the same effective mode and the primary entry stays the same
- **THEN** those flag changes do not invalidate queued queries

#### Scenario: Cancellation during synchronous work
- **WHEN** cancellation arrives after native semantic computation starts
- **THEN** the protocol response is cancelled once while that computation finishes safely and retains its admission permit until completion

#### Scenario: Request ID reused after cancellation
- **WHEN** the client reuses an ID after receiving its cancellation response
- **THEN** the earlier request cannot acquire the new request's abort registration or complete its response

#### Scenario: Stateful command cancellation
- **WHEN** a client cancels a lifecycle, execute-command, or unsupported extension request
- **THEN** the server ignores cancellation and allows the request to complete consistently

### Requirement: Source-only highlights remain responsive
The server SHALL answer document highlights for opened files from their synchronized source without waiting for semantic admission, while preserving highlight results and the existing conversion semantics for the negotiated position encoding.

#### Scenario: Semantic analysis is busy
- **WHEN** a highlight request targets an opened document while semantic analysis holds the admission permit
- **THEN** the highlight request uses the in-memory syntax tree without waiting for that permit

#### Scenario: Highlight request targets a closed file
- **WHEN** the requested document is absent from the open-file source map
- **THEN** the existing semantic request path remains available to resolve its source

#### Scenario: Non-ASCII text before highlighted syntax
- **WHEN** the client uses UTF-8 or UTF-16 positions and the source contains multibyte or supplementary characters
- **THEN** the cursor and returned highlight ranges use the same conversion helpers as the existing semantic request path for that negotiated encoding
- **AND** nested loops retain their existing highlight boundaries

### Requirement: Cache cleanup has bounded concurrency and recent retention
The server SHALL run at most one automatic global cache sweep at a time, coalesce pending sweeps, and evict unused old generations without changing compiled document output.

#### Scenario: Burst of compile results
- **WHEN** several compile results request cache maintenance during a sweep
- **THEN** the requests are coalesced into at most one pending follow-up sweep

#### Scenario: Edit and revert
- **WHEN** a document is edited and reverted across cache sweeps
- **THEN** its compiled output matches the same document compiled without those evictions

### Requirement: Package-index prefetch bounds workers and buffers reads
The server SHALL reserve at most one background index prefetch per registry and parse the preview package index with buffered upstream reads, preserving package metadata, preview namespace normalization, and parse/read error handling.

#### Scenario: Concurrent completion before the index is cached
- **WHEN** multiple completion requests encounter the same uncached package index
- **THEN** only one request reserves a background prefetch worker
- **AND** subsequent requests do not occupy extra blocking workers waiting for that prefetch

#### Scenario: Empty index is cached
- **WHEN** the registry already cached an empty index, including its existing error fallback
- **THEN** completion does not schedule another index prefetch

#### Scenario: Large index response
- **WHEN** completion prefetches an uncached package index from a blocking HTTP response
- **THEN** parsing reads chunks from that response instead of making one blocking read per JSON byte
- **AND** all valid package entries and their metadata remain available

#### Scenario: Malformed or interrupted index
- **WHEN** the index contains malformed JSON or its reader fails
- **THEN** the parser reports the corresponding error and the existing download fallback remains available

### Requirement: Late compilation status tolerates editor shutdown
The server SHALL tolerate a closed editor receiver when a background compilation reports its status, without panicking or aborting the process.

#### Scenario: Compilation outlives the editor receiver
- **WHEN** a compilation worker sends a status after the editor receiver closes
- **THEN** the failed send is handled like other closed-channel notifications
- **AND** the worker finishes without an uncaught panic
