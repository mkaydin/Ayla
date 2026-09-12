FROM node:22.15.0-bookworm@sha256:a1f1274dadd49738bcd4cf552af43354bb781a7e9e3bc984cfeedc55aba2ddd8 AS node
FROM rust:1.88.0-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules/npm /usr/local/lib/node_modules/npm
RUN ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s /usr/local/lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev liblzma-dev libbz2-dev libgtk-3-0 libnss3 libasound2 \
    libgbm1 libxss1 libxtst6 libdrm2 libx11-xcb1 xvfb xauth ca-certificates \
 && rm -rf /var/lib/apt/lists/*
ARG AYLA_BUILD_UID=1000
ARG AYLA_BUILD_GID=1000
RUN (getent group "$AYLA_BUILD_GID" >/dev/null || groupadd -g "$AYLA_BUILD_GID" builder) \
 && useradd -m -u "$AYLA_BUILD_UID" -g "$AYLA_BUILD_GID" builder \
 && mkdir /work /output /cache && chown "$AYLA_BUILD_UID:$AYLA_BUILD_GID" /work /output /cache /usr/local/cargo
USER builder
WORKDIR /work
ENV AYLA_PORTABLE_RELEASE=1 CARGO_HOME=/cache/cargo npm_config_cache=/cache/npm ELECTRON_CACHE=/cache/electron ELECTRON_BUILDER_CACHE=/cache/electron-builder
ENTRYPOINT ["bash", "-c", "cp /source/scripts/container-release.sh /tmp/ayla-release.sh && exec bash /tmp/ayla-release.sh"]
