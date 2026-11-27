# SPDX-License-Identifier: GPL-3.0-only
# The infrasonic web player as a small static web server image.
#
#   docker build -f crates/web/docker/Dockerfile -t infrasonic-web .
#   docker run -p 8080:8080 -e INFRASONIC_SERVER=https://music.example.org infrasonic-web
#
# The WebAssembly is the same on every platform, so it is built once on the build machine.
# Base images by digest (Dependabot keeps them current, .github/dependabot.yml).
FROM --platform=$BUILDPLATFORM rust:1-bookworm@sha256:75d26c23c24cc6ea1cdabf0714a6bff8fd7bf3248716043fd7ae86dac7f41b94 AS build
RUN apt-get update && apt-get install -y --no-install-recommends python3 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup show active-toolchain || rustup toolchain install
COPY Cargo.toml Cargo.lock ./
RUN v=$(sed -n '/^name = "wasm-bindgen"$/{n;s/version = "\(.*\)"/\1/p}' Cargo.lock) \
 && cargo install wasm-bindgen-cli --version "$v" --locked
COPY . .
ARG VERSION=dev
RUN OUT=/site VERSION=$VERSION scripts/web.sh \
 && find /site -type f ! -name config.js \( -name '*.wasm' -o -name '*.js' -o -name '*.css' \
      -o -name '*.html' -o -name '*.svg' -o -name '*.webmanifest' \) -exec gzip -9 -k {} +

FROM joseluisq/static-web-server:2-alpine@sha256:b3f7db31db9a68754205b3c0831fe5b066bd1daeb4f45aaa5007847a8c29ede2
LABEL org.opencontainers.image.title="infrasonic-web" \
      org.opencontainers.image.description="Web player for Subsonic and OpenSubsonic servers" \
      org.opencontainers.image.source="https://github.com/FionaPreroll/infrasonic" \
      org.opencontainers.image.licenses="GPL-3.0-only"
COPY --from=build /site /srv/infrasonic
COPY crates/web/docker/sws.toml /etc/infrasonic/sws.toml
COPY --chmod=755 crates/web/docker/entrypoint.sh /usr/local/bin/infrasonic-web
# Optional: a server address filled in on the login page, and a path prefix when the player
# sits behind a reverse proxy at e.g. https://example.org/music/.
ENV INFRASONIC_SERVER="" \
    INFRASONIC_BASE_PATH=""
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s CMD wget -qO- http://127.0.0.1:8080/health || exit 1
ENTRYPOINT ["/usr/local/bin/infrasonic-web"]
