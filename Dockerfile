# Web UI
FROM node:22-alpine AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# Static binary with the UI embedded
FROM rust:1.91-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
# rust-toolchain.toml is left out on purpose: the image already has 1.91.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY --from=web /web/dist ./web/dist
RUN cargo build --release --locked

FROM scratch
COPY --from=build /src/target/release/lorvaq /lorvaq
USER 65534:65534
EXPOSE 8080
ENTRYPOINT ["/lorvaq"]
CMD ["server", "--demo", "--listen", "0.0.0.0:8080"]
