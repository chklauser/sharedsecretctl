FROM lukemathwalker/cargo-chef:0.1.59-rust-1.69.0-slim AS chef
WORKDIR /usr/src/app

ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV CARGO_INCREMENTAL=false

##################################################################################################
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

##################################################################################################
FROM chef as build

COPY --from=planner /usr/src/app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --bin sharedsecretctl

##################################################################################################
FROM debian:bullseye-slim as run
COPY --from=build /usr/src/app/target/release/sharedsecretctl /usr/local/bin/sharedsecretctl
USER 999:999
ENTRYPOINT ["/usr/local/bin/sharedsecretctl"]
