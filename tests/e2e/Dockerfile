FROM rust:1-bookworm
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      openvpn openssl iproute2 iputils-ping nftables dnsutils procps python3 libdbus-1-dev pkg-config obfs4proxy \
    && rm -rf /var/lib/apt/lists/*
