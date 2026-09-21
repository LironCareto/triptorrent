FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p triptorrent-cli

FROM debian:bookworm-slim
RUN useradd --create-home --uid 10001 triptorrent
COPY --from=build /src/target/release/triptorrent /usr/local/bin/triptorrent
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/triptorrent"]
