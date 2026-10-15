FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates curl build-essential libgtk-4-dev libadwaita-1-dev libdbus-1-dev pkg-config \
      xvfb xauth dbus dbus-x11 imagemagick x11-apps xdotool openbox fonts-cantarell adwaita-icon-theme librsvg2-common \
      openvpn openssl iproute2 iputils-ping nftables procps obfs4proxy tinyproxy dpkg-dev rpm librsvg2-bin \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component rustfmt,clippy
