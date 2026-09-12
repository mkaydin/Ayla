FROM archlinux:base-devel@sha256:061679925e5d5028f2df9395d441ce6c4b9accf0157ff3486c06b6962e014ef1
RUN pacman -Syu --noconfirm --needed git rust nodejs npm pkgconf openssl xz bzip2 zstd brotli electron41 namcap xorg-server-xvfb
ARG AYLA_BUILD_UID=1000
ARG AYLA_BUILD_GID=1000
RUN (getent group "$AYLA_BUILD_GID" >/dev/null || groupadd -g "$AYLA_BUILD_GID" builder) \
 && useradd -m -u "$AYLA_BUILD_UID" -g "$AYLA_BUILD_GID" builder \
 && mkdir /build /output /cache && chown "$AYLA_BUILD_UID:$AYLA_BUILD_GID" /build /output /cache
USER builder
WORKDIR /build
ENV CARGO_HOME=/cache/cargo npm_config_cache=/cache/npm ELECTRON_SKIP_BINARY_DOWNLOAD=1
ENTRYPOINT ["bash", "-c", "cp /source/scripts/container-arch.sh /tmp/ayla-arch.sh && exec bash /tmp/ayla-arch.sh"]
