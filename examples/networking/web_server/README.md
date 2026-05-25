# `web_server` — minimal HTTP/1.1 library

A small, dependency-free HTTP server library built on top of `of_io`.
Suitable for serving simple endpoints, health checks, JSON APIs, and
demos. One response per connection (`Connection: close`); concurrency is
serial — the user owns the accept loop.

## Files

- [`web_server.of`](./web_server.of) — the library: `Server`,
  `Connection`, `Request`, `Response`, and the HTTP parser.
- [`hello.of`](./hello.of) — example app: routes `GET /`,
  `GET /health`, `POST /echo`, and `404` everything else.
- [`of_io.of`](./of_io.of) — copy of the `of_io` facade so imports
  resolve next to the consumer (compiler convention).

## API

### `Server`

```of
struct Server { listener: TcpListener }

Server.bind(host: str, port: u16): Server | IoError
server.accept(): Connection | IoError
server.close()
```

`bind` sets `SO_REUSEADDR` so repeated dev runs don't trip over
`TIME_WAIT` from the previous instance.

### `Connection`

```of
struct Connection { stream: TcpStream }

connection.read_request(): Request | IoError
connection.write_response(resp: Response): IoError | null
connection.close()
```

`read_request` does a single `read()` of up to `MAX_REQUEST_BYTES`
(8 KiB), parses the request line, then frees the underlying buffer.
Body bytes that arrive in the same read are extracted into `req.body`;
headers are kept verbatim in `req.raw` for callers that need them.

### `Request`

```of
struct Request {
  method:   str,   // "GET", "POST", ...
  path:     str,   // "/", "/api/users", ...
  protocol: str,   // "HTTP/1.1"
  body:     str,   // "" for requests with no body
  raw:      str,   // the entire request including headers
}
```

### `Response`

Builders cover the common cases; `Response.with_status(...)` is the
escape hatch:

```of
Response.ok(body, content_type)          // 200 with explicit content type
Response.text(body)                      // 200 text/plain
Response.html(body)                      // 200 text/html
Response.json(body)                      // 200 application/json
Response.not_found()                     // 404
Response.bad_request(body)               // 400
Response.method_not_allowed()            // 405
Response.internal_error(body)            // 500
Response.with_status(code, text, body, content_type)
```

The response is serialized with `Connection: close`; we don't emit a
`Content-Length` (the language doesn't have a `str.size()` wired up
yet), so clients rely on EOF for framing.

## Usage

```of
import { println } from "of:core";
import { IoError } from "of_io";
import { Server, Connection, Request, Response } from "web_server";

function handle(req: Request): Response {
  if (req.method == "GET" && req.path == "/") {
    return Response.html("<h1>Hello</h1>\n");
  }
  if (req.method == "POST" && req.path == "/echo") {
    return Response.text(req.body);
  }
  Response.not_found()
}

function main(): i64 {
  var srv = Server.bind("127.0.0.1", 8090 as u16) as Server;
  println("listening on http://127.0.0.1:8090");
  while (true) {
    var conn = srv.accept() as Connection;
    var req = conn.read_request() as Request;
    conn.write_response(handle(req));
    conn.close();
  }
  0
}
```

## Build & run

From the repo root, after `cargo build -p otter_rt -p otter_io --release`:

```bash
cd examples/networking/web_server
../../../target/release/otter_fusion compile hello.of -o /tmp/hello.o
cc /tmp/hello.o \
   ../../../target/release/libotter_rt.a \
   ../../../target/release/libotter_io.a \
   -o /tmp/hello
/tmp/hello
```

Hit it from another terminal:

```bash
curl -i http://127.0.0.1:8090/
curl -i http://127.0.0.1:8090/health
curl -i -X POST -d 'ping' http://127.0.0.1:8090/echo
curl -i http://127.0.0.1:8090/anything-else   # 404
```

## Design notes

The interface is **explicit accept loop**, not express-style closures
or axum-style typed router. The language doesn't yet support the
features that would make those shapes work:

- Function literals (lambdas) compile but the validator stubs them to
  the `null` type, so `app.get("/", function(req): Response { ... })`
  can't be type-checked. Closures with captures need MIR/codegen
  support that doesn't exist.
- `List<T>` / `Map<K, V>` have language-level types and method
  signatures but no runtime — linking against them fails with
  `__of_list_*` / `__of_map_*` unresolved. So a route registration
  table isn't viable.
- Virtual interface dispatch returns null from `__of_vtable_lookup`,
  which crashes any `interface.method()` call. `List<Handler>` with
  dynamic dispatch is out for the same reason.
- Generic type parameters can't be called as methods at the use site
  (`A.handle()` for an unknown `A`), so a `Server<App>` shape that
  delegates to `app.handle(req)` doesn't pass the validator.

Given those constraints, the API above (user owns the loop, user
writes a `handle(req)` dispatch function, library provides the I/O +
parsing primitives) is the smallest shape that actually compiles and
runs. When the language ships lambdas + virtual dispatch, the same
`Server` / `Connection` pieces will support an express-style router
on top — the underlying I/O won't have to change.

## Limitations

- Only the first read off the socket is parsed; requests larger than
  `MAX_REQUEST_BYTES` (8 KiB) are truncated.
- Headers are not split into a map — `req.raw` holds them verbatim;
  the language has no `str.split` or `str.index_of` wired up to a
  runtime impl yet, so manual scanning is the way until those land.
- `Content-Length` is not emitted on responses; the response is
  delimited by closing the connection. Browsers and `curl` cope, but
  HTTP/1.1 keep-alive clients won't.
- One request per connection. Pipelining and keep-alive need a more
  involved I/O loop than the language's blocking `read` allows today.
