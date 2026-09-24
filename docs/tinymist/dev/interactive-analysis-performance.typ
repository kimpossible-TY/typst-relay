#set document(title: "Interactive Analysis Performance Investigation")
#set page(paper: "a4", margin: 20mm)
#set text(size: 10pt)
#set heading(numbering: "1.")

= Interactive analysis performance

Investigation date: 23 September 2026. Scope: the Flow server on an
8 GiB macOS arm64 machine, using a saved private Typst book and the original
installed packages. The document itself is not distributed.

== Diagnosis

Selecting a server with `tinymist.serverPath` changes the executable launched
by the extension. It does not itself reduce compiler or analysis work. In a
VS Code Tunnel session, this path belongs to the machine running the remote
extension host. Workspace settings override remote settings; check the
project's `.vscode/settings.json` when a remote setting appears ineffective.
See #link("https://code.visualstudio.com/docs/configure/settings#_settings-precedence")[VS Code setting precedence].

The earlier static dependency-discovery patch left another execution path:
resolving an import through nested module fields could fall back to runtime
tracing. The investigated dependency was Fletcher's import of `deps.cetz.draw`.
Semantic-token requests executed two full traces per edit even though the
module exports were available statically.

A separate completion request targeted `s.tag` inside a contextual callback.
Its runtime value was only available during layout. Eliminating unnecessary
import traces does not eliminate that contextual computation.

The 22 September baseline used a fresh server for each scenario. Its repeated
heading edits took 4.56--4.72 seconds to compile. Semantic requests took 9.74
and 8.64 seconds; contextual completion took 4.88 and 5.69 seconds. The analysis
scenarios exceeded a 5 GiB physical-footprint guard during the third edit.
Compile-only footprint rose from 3.00 to 4.93 GB over three edits. These are
individual observations, not distribution estimates or proof of a memory leak.

A sample from a separate, much longer server session showed compiler and trace
threads waiting for cache read locks while eviction held a write lock and
destroyed cached values. This establishes a possible contention path. The
sample does not establish the exact duration spent waiting for each lock.

== Implemented server changes

- Resolve module-field imports through intermediate re-exports, preserve
  implicit import names, and stop recursive alias traversal. Regression tests
  cover bare, selected, wildcard, renamed, and nested selected imports.
- Complete proven static values and module surfaces without execution. Preserve
  runtime fallback for mutable records, active styles, and ambiguous closure
  values. When ordinary evaluation already observes the first runtime value,
  completion can avoid the subsequent layout while preserving that observation.
- Cancel supported read-only requests and discard superseded queued snapshots.
  Native synchronous semantic work uses a blocking worker and one admission
  permit. Already-running work keeps that permit until it finishes; cancellation
  does not forcibly interrupt Typst or release shared state prematurely.
  Effective pin/unpin transitions also invalidate queued snapshots even when
  the primary entry file stays the same.
- Read opened-document highlights directly from synchronized syntax, so they do
  not wait behind contextual completion. Preserve the existing disk-source
  fallback for closed documents and the negotiated position conversion.
- Buffer HTTP reads while parsing the package index and reserve one background
  prefetch per registry. An active-thread sample found unbuffered JSON parsing
  crossing the blocking HTTP bridge for individual bytes; this kept a native
  test runtime alive for minutes after its assertions finished. Dedicated tests
  verify bounded upstream reads, preserved metadata/errors, and one prefetch
  owner under concurrent requests.
- Tolerate a closed editor status channel after shutdown. A reproduced native
  panic previously escaped a background compilation worker and caused Rayon's
  process-wide abort. The handler now treats this send like diagnostic sends;
  a worker regression covers the closed receiver explicitly.
- Coalesce automatic global cache sweeps into one worker with at most one pending
  follow-up. Retain entries for one unused sweep interval instead of ten.
  This is a cache-generation policy, not a memory limit. Source and VFS cache
  retention policies remain separate.

Request cancellation applies to `textDocument/*`, `workspace/symbol`, and
`workspace/willRenameFiles`. Lifecycle, execute-command, and other extension
requests finish their state transitions. Responses are tied to request identity,
so late completion cannot answer a new request that reuses a cancelled ID.

== Measurement method

Use separate preserved baseline and candidate release executables. Pin
`main.typ`, open the same chapter with a controlled unused `s.tag` binding,
and repeat the same heading edits. Run compile-only, semantic-token, completion,
and mixed-request scenarios in separate server processes. Preserve the original
document and package files; do not substitute the earlier diagnostic Fletcher
alias modification into these comparisons.

Record exact LSP responses and semantic-token legends as well as response
latency, compilation messages, completed full-trace/evaluation counts, and
physical footprint with a nominal 0.25-second polling delay and recorded sample
timestamps. The actual median sample gap was about 0.32 seconds in both
compile-only trials. Stop a trial above 5 GiB rather than
allowing the experiment to cause unrestricted swap growth. Run at least twelve
sequential edits where that guard permits. Compare the overlapping completed
responses with the baseline, and report early termination explicitly.

Cache logs distinguish comemo cleanup, source cleanup, and VFS cleanup.
Semantic admission logs measure time waiting for the query permit. Neither
measurement isolates time blocked inside individual comemo locks. No preview
WebSocket transmission or client-side rendering latency is measured by this
standalone LSP experiment.

The trials ran on a shared machine. During the historical intermediate mixed
trial, the existing editor's old server still accounted for about 17.2 GiB of process
footprint while the system reported about 12 GiB of swap in use. Its resident
memory was small; footprint must not be read as physical RAM resident at that
instant. That process was not restarted by the experiment. This observation
does not establish identical background pressure during the baseline or prove
that swapping caused a particular latency outlier.

== Final repeated-edit replay

The final binary, SHA-256 starting `bd837663`, completed all 12 requested
rounds in each of four isolated LSP sessions using the original book and
packages. A fresh run of the preserved baseline, SHA-256 starting `62fb0d19`,
was measured immediately afterwards. The baseline reached the diagnostic
5 GiB process-footprint guard in every mode. Both runs preserved all 2,002
hashed source/configuration inputs, and every completed compilation reported
281 pages. This replay does not test preview transport or PDF equivalence.

#table(
  columns: (1.2fr, 1fr, 1fr, 1fr),
  inset: 5pt,
  table.header([Mode], [Baseline completed rounds], [Final completed rounds], [Final peak footprint]),
  [Compile], [3 / 12], [12 / 12], [3.674 GiB],
  [Semantic tokens], [2 / 12], [12 / 12], [3.679 GiB],
  [Contextual completion], [2 / 12], [12 / 12], [4.293 GiB],
  [Mixed requests], [1 / 12], [12 / 12], [4.734 GiB],
)

Baseline peaks were 5.012--5.096 GiB because the guard samples periodically;
it is not an instantaneous allocation limit. Actual sampling gaps had per-mode
medians of 0.258--0.260 seconds and a maximum of 0.264 seconds across the fresh
pair. Twelve completed rounds demonstrate this bounded workload only. Some
round-end footprints still increased, so the result does not establish a byte
cap, indefinite stability, or the absence of a leak.

The following latency comparisons include only rounds completed by both
binaries. Mixed requests have only one overlapping completed round; their
values are observations from that round, not a distribution estimate.

#table(
  columns: (1.8fr, .6fr, .9fr, .9fr),
  inset: 5pt,
  table.header([Measurement], [Rounds], [Baseline median], [Final median]),
  [Compile after edit], [3], [4.385 s], [4.408 s],
  [Semantic tokens], [2], [7.994 s], [0.394 s],
  [Contextual completion], [2], [4.558 s], [4.870 s],
  [Mixed: latest semantic tokens], [1], [25.170 s], [4.560 s],
  [Mixed: latest completion], [1], [11.395 s], [10.048 s],
  [Mixed: document highlight], [1], [3.326 ms], [3.777 ms],
)

Semantic-token latency decreased by 95.1% in the overlapping rounds. Normal
compilation was approximately unchanged, and contextual completion was 6.8%
slower. Across all 12 final rounds, medians were 4.435 seconds for compilation,
0.078 seconds for semantic tokens, and 5.187 seconds for contextual completion.
Mixed-request medians were 6.284 seconds for latest semantic tokens,
12.400 seconds for latest completion, and 3.990 milliseconds for highlighting.
There is no uncensored baseline for those later rounds.

All eight available successful query payload comparisons against the fresh
baseline matched after canonicalization: seven belong to common completed
rounds, and one is the next round's early highlight response. Semantic-token
data and legends matched; canonicalization ignores only the opaque token
result identifier and completion item order while retaining item fields.
All 60 successful query payloads also matched the intermediate `9ad02cb4`
candidate. All 283 files in the original baseline/intermediate result sets
remained unchanged.

The fresh baseline recorded four full expression traces in two semantic-token
rounds; the final binary recorded zero full traces and zero evaluation attempts
across twelve. Contextual completion still required one full trace per round:
the final run recorded twelve evaluation attempts and twelve full traces.
The mixed final run recorded 24 of each, versus seven full traces in the
baseline's one completed round. These counters describe completed-round
checkpoints; interrupted work after a guard may be absent.

All 24 requested cancellations received exactly one cancellation response;
all twelve deliberately stale queued requests received `ContentModified`.
There were no duplicate or unexpected terminal responses. Cancellation
notification-to-response time had a median of 0.311 milliseconds and a maximum
of 2.735 milliseconds. This acknowledges cancellation promptly; computation
already running can continue until it releases the semantic permit. In mixed
requests, admission waits reached 7.769 seconds. Comemo cleanup medians were
0.572--1.012 seconds across the four final modes. These cleanup durations
include acquisition and cleanup work; admission timings do not isolate
internal comemo lock waits.

Shared-machine conditions changed before this final replay: the old editor
server process previously observed at approximately 17 GiB was no longer
present, and system swap was about 2.2--2.7 GiB rather than approximately
12.4 GiB during the intermediate run. This investigation did not stop that
process. Consequently, intermediate-to-final latency differences cannot be
attributed solely to the HTTP buffering/prefetch correction. The fresh
baseline is the primary comparison, but it is still one sequential run per
mode on a shared machine, not a randomized controlled benchmark.

The raw wire logs, source manifests, memory samples, and comparisons are in
`.local/optimization-20260923/final-candidate-results`,
`final-baseline-results`, and `final-comparison`. The exact command, binary
hashes, and before/after pressure observations are recorded in
`final-replay-manifest.json`.

== Historical intermediate replay

The preserved baseline is `62fb0d19…`; this earlier timed candidate is
`9ad02cb4…`. It predates the final pin-mode, HTTP buffering/prefetch, and
shutdown fixes. These results are retained as historical observations; the
final paired replay is reported separately. All trials below use the original
installed document packages.

#table(
  columns: (1.5fr, 1fr, 1fr, 1fr, 1fr),
  inset: 5pt,
  table.header([Scenario], [Baseline edits], [Candidate edits],
    [Baseline peak], [Candidate peak]),
  [Compile only], [3], [12], [5.040 GiB], [3.565 GiB],
  [Semantic tokens], [2], [12], [5.061 GiB], [3.809 GiB],
  [Contextual completion], [2], [12], [5.019 GiB], [4.215 GiB],
  [Mixed requests], [1], [12], [5.076 GiB], [4.513 GiB],
)

Every baseline trial exceeded the 5 GiB guard during a subsequent edit. Every
candidate trial completed twelve edits without that guard firing. Sampling
allows a small overshoot before termination. Twelve edits demonstrate this
workload's improved retention, not indefinite memory stability or a byte bound.

Semantic-token response median fell from 10.319 seconds over two completed
baseline edits to 88.5 milliseconds over twelve candidate edits. Comparing only
the same first two edits gives 375.2 milliseconds for the candidate, including
its first 663.8-millisecond request. Full document traces fell from two per edit
to zero. Semantic legends and every comparable successful token response match.

Ordinary compilation is not uniformly faster. Compile-only edited-compilation
median is 5.267 seconds for the baseline's three completed edits and 5.365
seconds for the candidate's twelve; the candidate's same first three edits
have a 5.453-second median. Contextual `s.tag` completion still requires one
full trace per request: matched first-two response medians are 5.754 versus
6.462 seconds, and the candidate's twelve-response median is 7.553 seconds.
The evaluation attempt accounted for only about 0.120 seconds through eight
rounds, compared with 57.3 seconds of full tracing; it is not the main cost.

In the mixed trial, the candidate returned 24 requested cancellations and
rejected twelve superseded queued queries. Cancellation response latency after
notification had a 0.949-millisecond median and a 41.266-millisecond maximum.
Highlights stayed responsive at a 3.861-millisecond median. The first latest
semantic request improved from 28.277 to 8.586 seconds, but the first latest
completion worsened from 15.494 to 17.999 seconds. Candidate twelve-round
medians were 8.353 and 17.058 seconds respectively. Already-running contextual
work still holds admission until it finishes, so cancellation bounds queued
work without eliminating that wait.

Automatic comemo sweep medians were 0.729, 0.893, 1.010, and 1.307 seconds
across the four candidate scenarios, with a maximum of 3.099 seconds. Source
cleanup stayed below 7.2 milliseconds and VFS cleanup below 0.6 milliseconds.
Mixed semantic admission wait reached 10.633 seconds. These are cleanup and
admission measurements, not direct measurements of comemo lock acquisition.
Long cleanup stalls therefore remain possible under this retention policy.

All eight comparable successful response pairs match after removing opaque
semantic-token handles and canonicalizing completion order. The mixed
comparison includes one highlight received in the baseline's otherwise
incomplete second round. All 2,002 checked source/package files are unchanged.
The original package's layout-convergence warnings remain in this server-only
comparison. Raw wire messages, memory samples, response hashes, timing logs,
binary hashes, and source manifests are preserved in the local investigation
artifacts.

== Full-book output verification

A separate full-book comparison used the original packages with baseline
`62fb0d19` and final candidate `bd837663`. Both outputs have 281 pages;
extracted text and word coordinates rounded to 0.001 pt match. Rasterizing
every page at 96 ppi produced identical pixels. PDF bytes differ, so byte
identity is not claimed. Inputs and package files remained unchanged.

The single cold compilations took 10.07 and 9.28 seconds respectively, with
peak process footprints of 2.496 and 2.488 GiB. These one-off timings do not
supersede the repeated-edit comparison above. Both retained the two original
layout-convergence warnings associated with position queries in the local
annotation package.

The package-only comparison then held the final server fixed and changed only
`scoped-annotations` 0.3.0. The patch removes a hidden second canvas and the
query of its reference-label position. An empty content anchor fixes the canvas
origin, while floating drawing elements preserve the visible overlay. All
281 pages again match at 96 ppi, with identical text and word coordinates to
0.001 pt. Both the non-convergence and unstable-sequence-position warnings
disappear. The single cold runs took 9.544 versus 9.301 seconds; this is not
evidence of a substantial or repeatable speed improvement.

After these isolated comparisons, the verified package file was applied to
both its source repository and installed local-package copy. Their original
files were backed up first. The repeated-edit server results above predate
this application and use the original package, so they do not silently combine
server and package changes.

A final compile through the actual installed package directory, without the
isolated candidate package root, produced 281 pages with no warnings. Its text
and word coordinates match the verified package candidate, confirming that
the installed copy is the tested one. Source and installed package hashes
match, and no inputs changed during this final application check.

== Local application

The final release executable is available at
`/Users/taeyoung/Developer/tinymist/target/release/tinymist`. The existing remote
Machine setting already selects this path. The book workspace still selected
the old executable under `versions/288b215d`; that single overriding key was
removed after validation, preserving every other setting. Reload the remote
VS Code window with `Developer: Reload Window` to activate the new executable.
This investigation did not terminate the user's editor or server process.

The application record and the previous workspace settings and package files
are in `.local/optimization-20260923/application-manifest.json` and
`applied-backups/`. The package change is also retained as a standalone patch
under `.local/investigation-20260923/scoped-annotations/`. No book source text
was edited.

== Correctness and limits

The updated query suite passes 89 tests, including completion and definition
snapshots. The server library passes 59 tests with three existing ignored tests;
the cancellation suite passes twelve tests. The six queue regressions cover cancellation, preview memory events, other
input changes, and unchanged compile results; pin-mode and late-status
regressions cover context transitions and editor shutdown. The project suite passes nineteen
tests with five existing ignored filesystem-watcher tests. Cache tests verify
coalescing and edit/revert geometry preservation, not a byte-size bound.
The package suite adds four focused buffering/prefetch regressions. Native
workspace-change fixtures use a seeded empty package index to keep their
behavior independent of network access; the production parser is tested
separately with both successful and failing readers.

One global semantic permit prevents a pile-up of heavy work, but semantic
requests can still wait behind contextual completion that has already started.
Opened-document highlights bypass that admission because they only inspect
syntax. Snapshot
creation also occurs before admission and can acquire shared cache locks.
Custom commands outside the semantic queue are not covered by that queue.
The web build retains synchronous execution rather than using native blocking
threads. Feature compilation alone does not establish browser responsiveness.

Recorded editor requests are replayed with response barriers and at most five
retries of `ContentModified`; raw responses and transient errors are retained.
Comparison of 113,400 successful baseline/candidate responses found four
intentional completion additions in each VS Code scenario: statically resolved
modules now offer valid postfix snippets while preserving their exported fields
and signatures. Other differences were opaque semantic-token handles and
reference-list ordering. Token payloads remain unchanged; snapshot normalization
preserves token-handle equality relationships. The production `--replay` command
is not changed by this test-client pacing.

Unicode highlight checks exposed an existing UTF-8 conversion defect in
`tinymist-analysis/src/location.rs`: the UTF-8 branches call Typst's
`line_column_to_byte` and `byte_to_column`, whose columns count Unicode scalar
values instead of bytes. For example, the prefix `한😀 ` occupies eight UTF-8
bytes but is reported as column three. The highlight optimization preserves
the existing conversion path; its tests check equivalence for both encodings
and independently verify UTF-16 ranges. Correcting the shared UTF-8 helpers
is a separate issue affecting other language features too.

Strict clippy, formatting, ten native/web feature combinations, and all nine
final CLI/editor integration tests pass. These checks establish the tested
configurations and document behavior; they do not establish client-side preview
rendering speed, internal cache-lock wait times, or indefinite memory stability.
