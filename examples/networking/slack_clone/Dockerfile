# syntax=docker/dockerfile:1.7
#
# Build context must be the repo root:
#   docker build -f examples/networking/slack_clone/Dockerfile -t slack_clone .

# ---------- builder ----------
FROM rust:1-bookworm AS builder

WORKDIR /src
COPY . .

# 1. Build the static runtime libs (otter_rt, otter_io).
# 2. Build the otter_fusion compiler binary.
RUN cargo build --release -p otter_rt -p otter_io \
 && cargo build --release --bin otter_fusion

# 3. Compile and link slack.of into a binary. `build` auto-finds libotter_rt.a
#    next to the compiler and adds -lpthread -ldl -lm on Linux; we only need
#    to pass libotter_io.a explicitly.
WORKDIR /src/examples/networking/slack_clone
RUN /src/target/release/otter_fusion build slack.of \
        -l /src/target/release/libotter_io.a \
        -o /tmp/slack

# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime

COPY --from=builder /tmp/slack /usr/local/bin/slack

EXPOSE 8090
CMD ["/usr/local/bin/slack"]
