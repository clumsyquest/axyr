# axyr-engined — the engine as a server (see src/bin/engined.rs).
# Build: docker build -t axyr-engined engine/
# Run:   docker run -e PORT=8080 -e AXYR_TOKEN=... -e AXYR_AGENT_LISTEN=off axyr-engined

FROM rust:bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends libudev-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --locked --bin axyr-engined

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends libudev1 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/axyr-engined /usr/local/bin/axyr-engined
# Cloud default: agents come in through the HTTPS edge (wss://host/agent);
# the plain TCP listener is off. PORT is injected by the platform.
ENV AXYR_AGENT_LISTEN=off
CMD ["axyr-engined"]
