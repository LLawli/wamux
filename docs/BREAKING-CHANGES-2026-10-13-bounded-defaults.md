# Breaking changes for the operator: bounded defaults (#80, 2026-10-13)

No `.proto` changes. Two defaults that used to be unbounded now have a limit.
Check them before upgrading.

| # | Change | Impact |
|---|--------|--------|
| 1 | `replay_max_event_bytes` defaults to 262144 (256 KiB), was 0 (no cap) | **BREAKING** (a very large event is no longer replayable) |
| 2 | The `method` metric label is `unknown` for a path the daemon does not serve | **BREAKING** for dashboards that filtered by an invalid path |

## 1. The replay ring has a size cap by default

**Before:** `replay_max_event_bytes` defaulted to `0`, meaning no cap. Any event
except history sync went into the replay ring, however large, so one account's
ring could hold 256 events of unbounded size.

**Now:** the default is `262144` (256 KiB). An event whose encoded size is above
it is still delivered live to every subscriber, but it is not kept in the ring,
so a subscriber that reconnects with `replay_from_ring` does not get it back.
History sync was already excluded and still is. With the default ring of 256
events, an account's ring is bounded at 64 MiB.

The default was chosen from 327 147 real envelopes: outside history sync the
largest was 146 KiB and none was above 256 KiB, so nothing seen live drops out
of the replay window.

**To migrate:** nothing, unless you relay events larger than 256 KiB and need to
replay them. Then raise `replay_max_event_bytes` (env
`WAMUX_REPLAY_MAX_EVENT_BYTES`), or set it to `0` for the old behaviour.

## 2. Metric labels come from known methods only

**Before:** `wamux_grpc_requests_total` and
`wamux_grpc_request_duration_seconds` used the raw request path as the `method`
label. A client that called made-up paths created one Prometheus series per
path, without limit.

**Now:** `method` is `pkg.Service/Method` only when the path is a method the
daemon serves (the `wamux.v1` services and gRPC reflection v1). Every other path
is labelled `unknown`, so the series count is bounded. The status label is
unchanged. The log line keeps `method` (the same label) and, for an unknown
path, adds `path` with the raw path cut at 128 characters.

**To migrate:** a dashboard or alert that selected an invalid path by its label
now selects `method="unknown"`. Known methods keep their labels. The
`grpc.reflection.v1alpha` service is not served and is `unknown`.
