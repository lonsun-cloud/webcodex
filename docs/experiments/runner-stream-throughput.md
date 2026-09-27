# Runner Stream Throughput Telemetry v1

## Purpose

Runner Stream Throughput Telemetry v1 adds bounded, payload-safe measurements to the
existing Server/Runner lifecycle so Direct-vs-Code-Mode experiments can distinguish
Server queueing, stream-channel pressure, Runner dispatch, Runner-local request
duration, and result return from the already measured model/host gaps.

This is an observability change only. It does not change Runner wire messages,
Runner scheduling, request batching, compression, WebSocket/QUIC selection, or
Code Mode semantics.

## Lifecycle and clock ownership

```text
Server process (Server monotonic clock)
  ToolRuntime
    -> RunnerRegistry enqueue
    -> per-Runner request queue
    -> registry dequeue by polling / streaming request pump
    -> streaming outgoing mpsc admission
    -> WebSocket or QUIC single writer send completion
                                           |
                                           | existing RunnerEnvelope protocol
                                           v
Runner process (Runner monotonic clock)     |
  stream receive                            |
    -> dispatch worker begins               |
    -> existing authoritative Runner-local request duration
    -> Result / JobUpdate outgoing mpsc admission
    -> WebSocket or QUIC single writer send completion
                                           |
                                           v
Server process (Server monotonic clock)
  stream receive
    -> Result / JobUpdate registry processing
    -> matching Result accepted
    -> ToolRuntime resumes
```

Server and Runner may run on different hosts. The implementation never subtracts a
Server timestamp from a Runner timestamp. No metric in this document is one-way
network latency.

## Metrics

All metrics use the repository's existing structured `runtime_metric` tracing
convention. Missing measurements are omitted rather than recorded as zero.
Metric emission is fail-open: telemetry panics are caught at the observation
boundary and cannot change request/result semantics.

### Server registry lifecycle

| Metric | Start -> end | Clock owner | Includes |
| --- | --- | --- | --- |
| `server_runner_request_queue_wait_seconds` | RunnerRegistry enqueue -> authoritative dequeue for a Runner transport | Server monotonic clock | Server-side request queue wait |
| `server_runner_request_round_trip_seconds` | RunnerRegistry enqueue -> matching ordinary Result accepted | Server monotonic clock | Server queueing, stream/polling transport, Runner dispatch/request processing, serialization, and result return |

The round-trip metric is intentionally not named network latency. The request's
transport is captured when it is dequeued, so a later reconnect cannot relabel the
result sample.

An undispatched/cancelled request emits neither latency metric. A result cannot
create a round-trip sample unless the corresponding request was authoritatively
dequeued first.

### Server streaming session

These metrics apply to the post-registration WebSocket and QUIC streaming sessions. The credential-bearing register/registered handshake is intentionally outside this telemetry window.

| Metric | Meaning |
| --- | --- |
| `server_stream_outgoing_channel_events_total` | Bounded outcome count for admission to the Server's single outgoing mpsc |
| `server_stream_outgoing_backpressure_total` | Actual `TrySendError::Full` observations; queue contents are never recorded |
| `server_stream_outgoing_channel_wait_seconds` | Request-pump ready -> successful outgoing mpsc admission, on the Server monotonic clock |
| `server_stream_outgoing_envelopes_total` | Envelope writer outcomes |
| `server_stream_writer_send_seconds` | Writer mpsc receive -> local encode/send completion, on the Server monotonic clock |
| `server_stream_incoming_envelopes_total` | Incoming envelope counts by bounded kind |
| `server_stream_ingress_processing_seconds` | Result/JobUpdate receive handler -> registry completion/rejection, on the Server monotonic clock |
| `server_stream_session_disconnects_total` | Streaming session teardown count |
The request pump preserves its canonical awaited `mpsc::send` semantics. Its
channel wait is measured directly; it is not probed with `try_send` merely to
classify backpressure. Therefore `server_stream_outgoing_backpressure_total`
comes only from pre-existing best-effort `try_send` control paths that actually
observe `TrySendError::Full`.

A writer latency sample is emitted only for successful sends. Transport/encoding
failure increments an outcome counter without manufacturing a successful duration.
Local writer send completion does not prove that the peer has read or processed the
frame.

### Runner request lifecycle

| Metric | Start -> end | Clock owner |
| --- | --- | --- |
| `runner_stream_request_dispatch_wait_seconds` | Request envelope accepted by the shared stream handler -> spawned dispatch worker actually begins | Runner monotonic clock |
| `runner_request_duration_seconds` | Existing authoritative Runner-local `duration_ms` carried by the result -> same duration projected as a runtime metric | Existing request-duration owner |

`runner_request_duration_seconds` does not start a second stopwatch. The existing
field is intentionally treated as request duration rather than pure process-execution
time: some request kinds include local preflight and not-started results may report
zero. If the result has no authoritative `duration_ms`, the metric is unavailable
and no zero sample is invented. The transport label can be `polling`, `websocket`,
or `quic`.

### Runner result/control return

| Metric | Meaning |
| --- | --- |
| `runner_stream_outgoing_channel_events_total` | Result/JobUpdate and regular control-plane admission outcome count, including keepalive, metadata, project inventory, and shutdown Goodbye |
| `runner_stream_outgoing_backpressure_total` | Actual `TrySendError::Full` observations from pre-existing non-blocking/best-effort paths |
| `runner_stream_outgoing_channel_wait_seconds` | Result/PersistentShellResult/blocking JobUpdate or bounded control send ready -> successful writer-mpsc admission, on the Runner monotonic clock |
| `runner_stream_outgoing_envelopes_total` | Single-writer send outcome count |
| `runner_stream_writer_send_seconds` | Writer mpsc receive -> local encode/send completion, on the Runner monotonic clock |
| `runner_stream_incoming_envelopes_total` | Incoming stream envelope counts by bounded kind |
| `runner_stream_session_disconnects_total` | Non-process-shutdown stream disconnect count |

Non-blocking JobUpdate delivery reports full/closed/success outcomes but does not
invent a wait duration. Result and JobUpdate remain separate envelope categories.
Best-effort control frames use the same bounded admission outcomes. A bounded
shutdown `Goodbye` send can additionally report `timeout`; that timeout is not
counted as backpressure unless an actual `TrySendError::Full` was observed.
Blocking Result, PersistentShellResult, and blocking JobUpdate submissions retain
their original `blocking_send` semantics. Telemetry measures their elapsed
admission wait but does not add a `try_send` probe, so the backpressure counter
never changes sender ordering merely to classify queue pressure.

## Cardinality and payload safety

Metric dimensions are deliberately closed:

- `transport`: `polling | websocket | quic` where the metric supports polling;
- `envelope_kind`: `request | result | job_update | persistent_shell_result |
  ping | pong | project_inventory | provider_metadata | goodbye | control`;
- `outcome`: a small fixed set such as `success | closed | backpressure |
  transport_error | rejected | timeout`.

Unknown/future envelope names collapse to `control`; they cannot silently create
new label values.

New metrics never label or serialize Project ids, Workflow Session ids,
ClientWindow values, request ids, Job ids, Runner/client/instance ids, tunnel or
host identities, paths, commands, arguments, stdout/stderr, result payloads,
credentials, or principals. The new throughput metrics use request ids only for
process-local lifecycle correlation and never emit them as metric labels; the
repository's pre-existing tool-request tracing correlation contract is unchanged.

## WebSocket and QUIC interpretation

WebSocket currently uses one persistent connection and one outgoing writer queue.
The Runner and Server may have multiple requests in flight, but all outgoing
envelopes pass through that writer.

QUIC v1 currently uses one QUIC connection, one bidirectional stream, and one
outgoing writer. The telemetry in this document is intended to establish whether
that single-stream/single-writer design creates material queue or writer pressure.
It does not assume that QUIC multistream would improve performance.

Polling shares the authoritative registry queue-wait and Server round-trip
measurements and can report existing Runner-local request duration. Stream-only
mpsc/writer metrics are not fabricated for polling.

## What remains unattributed

There is intentionally no cross-host one-way latency metric. With future paired
measurements, analysis may subtract known same-process components from the
Server-observed round trip and report the remainder only as an
**unattributed transport-and-queue residual**. That residual can include transport,
serialization, remote queueing, and other unmeasured work; it is not pure network
time.

v1 also does not separately measure kernel/TCP/QUIC buffering after local send
completion, peer receive scheduling before the application handler, or JobUpdate on
a dedicated stream because no such dedicated stream exists.

## Possible follow-up experiments

Telemetry may later justify isolated experiments such as RequestBatch/coalescing,
QUIC multistream, or a dedicated JobUpdate stream. Those are explicitly outside
this change and should be evaluated only after measurements demonstrate a material
bottleneck.
