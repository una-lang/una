# The Linux test environment of scripts/local-ci.sh --linux and --linux-arm64 and of
# scripts/hunt-linux.sh: the image of CI's ubuntu-24.04 runner (glibc 2.39, which the pinned Lean
# toolchain, glibc 2.29 or later, and the Z3 project's Linux executable, 2.38, need), for
# linux/amd64 or linux/arm64, with the Rust toolchain that rust-toolchain.toml pins. Every
# download is pinned by digest, but for the apt packages (below). It is not the image of the
# release (#2105): the Z3 library built into the shipped `una`, `una` and the Linux archive are
# built in scripts/local-ci/Dockerfile.release (AlmaLinux 8, glibc 2.28), where the limits leg
# also runs; the tests here link a Z3 library built with this image's toolchain from the same
# pinned source and recipe, whose digest the lock does not record.
# The multi-platform index of ubuntu:24.04: its linux/amd64 image is the one pinned before #241,
# sha256:f610ab94648195aa356059f5b41d6085c9d4d903c072430cdd1af7bdb646106b, and it has the linux/arm64 one.
FROM ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates cmake curl g++ gcc git libc6-dev make python3 time unzip \
    && rm -rf /var/lib/apt/lists/* \
    && git config --system --add safe.directory '*'
# git: the tests of scripts/release-gates.sh clone the checkout (#232), which the container
# mounts with another owner than its user, hence the safe directory.
# python3: the replay of the findings (#528), a step of scripts/local-ci.sh, converts the dumps
# with docs/reviews/formal-m2-replay/dumpcore.py.
# time: the POSIX `time -p` utility, which the scaling tests run their child under to read its
# processor time (#1656, crates/una-cli/tests/common/cpu.rs).
# cmake, g++, make: the Z3 build of scripts/build-z3.sh for the tests of this image. The apt
# packages carry no version and are not checked: supply-chain/z3.lock and
# scripts/check-toolchain.sh pin and check the release image's toolchain only (#2105).

# The container runs as an unprivileged user, never as root (#1394): the solver child refuses to
# run as root (ADR 0124 rule 7), so a root container fails every test that verifies. The user has
# a fixed uid and gid, 1000 (the image's own `ubuntu` user, which holds them, is replaced), and
# owns the toolchain's directories and the mount points of the volumes that scripts/local-ci.sh,
# scripts/hunt-linux.sh and scripts/local-release.sh mount, so that a new volume starts owned by
# it; a volume that a root container made before is handed over once by
# scripts/container-volumes.sh, which those scripts run first.
RUN userdel --remove ubuntu \
    && groupadd --gid 1000 una \
    && useradd --uid 1000 --gid 1000 --create-home --home-dir /home/una --shell /bin/sh una \
    && mkdir -p /opt/cargo/registry /opt/rustup /opt/z3 /opt/z3-cache /opt/lean /work/target \
    && chown -R 1000:1000 /opt/cargo /opt/rustup /opt/z3 /opt/z3-cache /opt/lean /work
USER 1000:1000
ENV HOME=/home/una

# The musl targets that rust-toolchain.toml lists (ADR 0124 step 4) are installed with the
# toolchain, so that no container run downloads them.
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
        --default-toolchain "$TOOLCHAIN" --component clippy --component rustfmt \
        --target x86_64-unknown-linux-musl --target aarch64-unknown-linux-musl \
    && rm /tmp/rustup-init

WORKDIR /work
