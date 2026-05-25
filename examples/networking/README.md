# Networking examples

Demonstrate the `of_io` external library: POSIX file, networking, and
multiplexing syscalls. Pair with `libotter_rt.a` + `libotter_io.a` at link
time. All examples use the high-level facade (`TcpListener`, `TcpStream`,
`FdSet`, `IoError`); the underlying `__of_*` externs are still declared in
`of_io.of` for callers that want the raw syscalls.

## Files

- [`of_io.of`](./of_io.of) — bindings + facade (a copy of `io/of_io.of`,
  kept here because the compiler resolves imports next to the consumer file).
- [`01_tcp_echo_server.of`](./01_tcp_echo_server.of) — single-connection
  blocking TCP echo server on `127.0.0.1:8080`.
- [`02_tcp_client.of`](./02_tcp_client.of) — connects to the server,
  sends `ping\n` via `write_str`, prints the reply.
- [`03_nonblocking_select.of`](./03_nonblocking_select.of) — uses
  `set_nonblocking(true)` + `select()` with a 2-second timeout on
  `127.0.0.1:8081`.
- [`04_looping_echo_server.of`](./04_looping_echo_server.of) — accepts
  many sequential clients on `127.0.0.1:8082` and echoes each connection
  until EOF.
- [`05_multi_message_client.of`](./05_multi_message_client.of) — sends
  three messages over one socket against `04`, prints the replies.

## Build

From the repo root:

```bash
cargo build -p otter_rt --release
cargo build -p otter_io --release
```

That produces `target/release/libotter_rt.a` and `target/release/libotter_io.a`.

## Compile an example

```bash
cd examples/networking
../../target/release/otter_fusion compile 01_tcp_echo_server.of -o /tmp/srv.o
cc /tmp/srv.o \
   ../../target/release/libotter_rt.a \
   ../../target/release/libotter_io.a \
   -o /tmp/srv
```

On Linux add `-lpthread -ldl -lm`.

## Run

Echo server + client, two terminals:

```bash
# Terminal 1
/tmp/srv

# Terminal 2
/tmp/cli           # built from 02_tcp_client.of the same way
```

Or with `nc`:

```bash
/tmp/srv &
echo "hello" | nc -w 1 127.0.0.1 8080
```

The select demo polls for 2 seconds, then exits:

```bash
/tmp/sel           # built from 03_nonblocking_select.of
# Optionally connect within 2s: `nc 127.0.0.1 8081 < /dev/null`
```

The looping server + multi-message client demo:

```bash
/tmp/04_looping_echo_server &
/tmp/05_multi_message_client    # sends "hello", "world", "goodbye"
kill %1
```
