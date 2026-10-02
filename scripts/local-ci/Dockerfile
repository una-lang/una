# The Linux environment of scripts/local-ci.sh --linux and scripts/local-release.sh: the image of
# CI's ubuntu-24.04 runner (glibc 2.39, which the pinned Z3 build needs), for linux/amd64, with
# the Rust toolchain that rust-toolchain.toml pins. Every download is pinned by digest.
FROM ubuntu:24.04@sha256:f610ab94648195aa356059f5b41d6085c9d4d903c072430cdd1af7bdb646106b

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl gcc libc6-dev unzip \
    && rm -rf /var/lib/apt/lists/*

# rustup-init, checked against the digest that static.rust-lang.org publishes with it.
ARG RUSTUP_VERSION=1.29.1
ARG RUSTUP_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
ARG TOOLCHAIN
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/opt/z3/bin:$PATH
RUN curl --fail --location --silent --show-error --retry 3 --output /tmp/rustup-init \
        "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/x86_64-unknown-linux-gnu/rustup-init" \
    && echo "$RUSTUP_SHA256  /tmp/rustup-init" | sha256sum --check \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal \
        --default-toolchain "$TOOLCHAIN" --component clippy --component rustfmt \
    && rm /tmp/rustup-init

WORKDIR /work
