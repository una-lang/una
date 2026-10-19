# The Linux environment of scripts/local-ci.sh --linux and --linux-arm64 and of
# scripts/local-release.sh: the image of CI's ubuntu-24.04 runner (glibc 2.39, which the pinned
# Z3 builds need), for linux/amd64 or linux/arm64, with the Rust toolchain that
# rust-toolchain.toml pins. Every download is pinned by digest.
# The multi-platform index of ubuntu:24.04: its linux/amd64 image is the one pinned before #241,
# sha256:f610ab94648195aa356059f5b41d6085c9d4d903c072430cdd1af7bdb646106b, and it has the linux/arm64 one.
FROM ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl gcc git libc6-dev python3 unzip \
    && rm -rf /var/lib/apt/lists/* \
    && git config --system --add safe.directory '*'
# git: the tests of scripts/release-gates.sh clone the checkout (#232), which the container
# mounts with another owner than its user, hence the safe directory.
# python3: the replay of the findings (#528), a step of scripts/local-ci.sh, converts the dumps
# with docs/reviews/formal-m2-replay/dumpcore.py.

# rustup-init for the image's architecture (TARGETARCH, which `docker build --platform` sets),
# checked against the digest that static.rust-lang.org publishes with it.
ARG RUSTUP_VERSION=1.29.1
ARG TARGETARCH
ARG TOOLCHAIN
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/opt/z3/bin:$PATH
RUN case "$TARGETARCH" in \
        amd64) host=x86_64-unknown-linux-gnu \
            sha256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71 ;; \
        arm64) host=aarch64-unknown-linux-gnu \
            sha256=15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433 ;; \
        *) echo "no rustup-init pinned for $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && curl --fail --location --silent --show-error --retry 3 --output /tmp/rustup-init \
        "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/$host/rustup-init" \
    && echo "$sha256  /tmp/rustup-init" | sha256sum --check \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal \
        --default-toolchain "$TOOLCHAIN" --component clippy --component rustfmt --component llvm-tools \
    && rm /tmp/rustup-init

WORKDIR /work
