# `slack_clone` — a tiny Slack-style chat server

A single-binary HTTP chat server: HTML page, channel list, message
post, and **HTTP long-polling** for near-real-time delivery. Built on
the existing `web_server` + `of_io` primitives, with one reactor thread
that multiplexes everything through `select(2)`.

## Files

- [`slack.of`](./slack.of) — entry point. Owns the event loop, routes
  HTTP requests to handlers, and threads the `Reactor` through to
  `route()` so handlers can park connections as long-poll waiters.
- [`reactor.of`](./reactor.of) — `ConnState`, the connection table, and
  per-connection read/write helpers. One `ConnState` per accepted
  client, in one of three phases: `READING`, `WRITING`, `WAITING`.
- [`state.of`](./state.of) — channels, messages, and waiters as
  hand-rolled linked lists (the runtime has no `List<T>` / `Map<K,V>`
  yet).
- [`url.of`](./url.of) — query-string and form-body parser using
  `Buffer` byte scans (no `str.split` either).
- [`json.of`](./json.of) — string-escape helper for the JSON we emit.
- [`views.of`](./views.of) — the HTML page, inline CSS, and a small JS
  long-poll client.
- [`web_server.of`](./web_server.of) / [`of_io.of`](./of_io.of) — copied
  verbatim from `../web_server/`. `of_io.of` also gains a `now_ms()`
  wrapper around the new `__of_now_ms` extern (see below).

## Architecture

```
                ┌─── reactor tick (single thread) ───┐
 browser  HTTP  │                                    │
   ┌──┐ ───────►│  select(listener + R-set + W-set,  │
   │JS│ ◄───────│         timeout=min-deadline)      │
   └──┘         │   ├─ listener ready → accept       │
                │   ├─ readable conn → read+dispatch │
                │   ├─ writable conn → drain+close   │
                │   └─ now ≥ deadline → expire poll  │
                │                                    │
                │   state: channels, messages,       │
                │          waiters list              │
                └────────────────────────────────────┘
```

Every accepted client is non-blocking. Until a request is fully read,
the conn sits in `READING` and is selected for readability. When the
handler returns a `Response`, the conn switches to `WRITING` and is
selected for writability. A `/poll` handler can instead park the conn
as `WAITING` — it stops being in either fd_set until either:

- a `POST /send` to its channel pushes the high-water mark above its
  `since` cursor, or
- the kernel time crosses its deadline.

In both cases the reactor flips the conn back to `WRITING` with a
prebuilt JSON response.

## Endpoints

| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/` | — | HTML page (sidebar + feed + composer) |
| GET | `/health` | — | `ok\n` |
| GET | `/channels` | — | `{"channels":[...]}` |
| POST | `/channels` | `channel=foo` | 201, creates if absent |
| POST | `/send` | `channel=...&user=...&text=...` | 204, appends + wakes waiters |
| GET | `/poll` | `?channel=X&since=N&wait=25` | suspends until new messages or deadline |

Form bodies are `application/x-www-form-urlencoded`. `since` is the
last message id the client already has; messages with `id > since` are
returned. `wait` is the long-poll deadline in seconds (capped at 30).

## Building

```bash
cargo build --release -p otter_rt -p otter_io
cargo build --release --bin otter_fusion

cd examples/networking/slack_clone
../../../target/release/otter_fusion compile slack.of -o /tmp/slack.o
cc /tmp/slack.o \
   ../../../target/release/libotter_rt.a \
   ../../../target/release/libotter_io.a \
   -o /tmp/slack
/tmp/slack
```

Open <http://127.0.0.1:8090/> in a browser. The page prompts for a
name (stored in `localStorage`), then opens a long-poll loop. Two
windows on the same machine see each other's messages within ~10ms.

`curl` for the API:

```bash
curl -s http://127.0.0.1:8090/health
curl -s http://127.0.0.1:8090/channels
curl -s -X POST -d 'channel=random' http://127.0.0.1:8090/channels
curl -s -X POST -d 'channel=general&user=alice&text=hello' http://127.0.0.1:8090/send
curl -s 'http://127.0.0.1:8090/poll?channel=general&since=0&wait=25'
```

## Runtime extension

The reactor needs a monotonic clock for waiter deadlines. We added one
helper to `io/src/lib.rs`:

```rust
pub extern "C" fn __of_now_ms() -> i64 {
    // clock_gettime(CLOCK_MONOTONIC) → ms
}
```

`of_io.of` here exposes it as `now_ms(): i64`. Nothing else in the IO
runtime changed.

## Why the design looks like this

Several language features that would normally shape a chat server
aren't wired up yet, so the design routes around them:

- **No `List<T>` / `Map<K,V>` runtime.** Channels, messages, waiters,
  and even the close-deferred-fd list are linked-list structs with
  `next: Self | null` fields. Walking is O(n) but n is small here.
- **No `str.split` / `str.index_of` / `str.size`.** Query and form
  parsing converts the string to a `Buffer` once, then byte-scans for
  `?`, `&`, `=`, `%`. JSON escaping is the same shape.
- **No closures or virtual dispatch.** The reactor doesn't take a
  handler callback; instead, the event loop lives next to `route()` in
  `slack.of` and calls it directly.
- **`return X;` at the end of an imported function trips the codegen
  verifier.** Final returns are written as implicit expressions
  (`r` instead of `return r;`) throughout — see the helpers in
  `url.of` / `state.of`.

## Limitations

- Requests bigger than 8 KiB are dropped (`read_buf` is fixed-size).
- The browser side does not URL-encode anything except via the
  built-in `encodeURIComponent` — fine for ASCII chat.
- No persistence. Restart loses every message and channel.
- No auth. Username is whatever the JS sends.
- One reactor thread; a slow handler (e.g. a huge `poll_json`) blocks
  everyone for that tick.
- `Content-Length` is not emitted on responses; framing is
  `Connection: close`. Browsers handle this fine but it means a
  request-per-connection.
