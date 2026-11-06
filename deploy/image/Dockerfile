# Demi's container image: one release's assets, from which
# `demi-server container` sets up or upgrades the server on the volume at
# /opt/demi (crates/server/src/container.rs). The release workflow builds it
# from the release's published assets, in a context laid out as
#
#   assets/<arch>/SHA256SUMS                          of these two
#   assets/<arch>/demi-<version>-server-linux-<arch>.tar.zst
#   assets/<arch>/demi-<version>-image-linux-<arch>.tar
#   bin/<arch>/demi-server                            the release's
#
# for amd64 and arm64.
FROM ubuntu:26.04

ARG TARGETARCH
# The update channel a new server starts on, as Settings › Updates shows it.
ARG CHANNEL=canary

# What the machine manager runs, as setup installs it on a machine
# (crates/server/src/setup.rs), and the roots TLS trusts.
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates e2fsprogs xfsprogs libarchive-tools nftables \
    && rm -rf /var/lib/apt/lists/*

# The users setup makes, with ids of their own, since the volume's files keep
# them from one image to the next.
RUN groupadd --system --gid 990 demi-cloud \
    && groupadd --system --gid 991 demi \
    && useradd --system --uid 991 --gid demi --groups demi-cloud \
      --home-dir /opt/demi/data/backend --no-create-home \
      --shell /usr/sbin/nologin demi

COPY assets/${TARGETARCH}/ /usr/lib/demi/assets/
COPY --chmod=0755 bin/${TARGETARCH}/demi-server /usr/lib/demi/demi-server
# As setup links it on a machine, for `docker exec ... demi-server status`.
RUN ln -s /opt/demi/current/bin/demi-server /usr/local/bin/demi-server

ENV DEMI_UPGRADE_CHANNEL=${CHANNEL} \
    DEMI_BACKEND_LISTEN=0.0.0.0:3271 \
    DEMI_BACKEND_DATA=/opt/demi/data/backend \
    DEMI_MANAGED_DATA=/opt/demi/data/cloud

EXPOSE 3271
VOLUME /opt/demi
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/lib/demi/demi-server", "container"]
