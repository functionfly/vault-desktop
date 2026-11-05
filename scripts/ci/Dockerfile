# Portable CI container — the Linux gate set in a reproducible image.
#
# Usage (from the repo root):
#
#   docker build -f scripts/ci/Dockerfile -t vault-ci .
#   docker run --rm -v "$PWD:/src" -w /src vault-ci
#
# The image carries the Linux Tauri build dependencies, so the `build`
# job (`bunx tauri build --no-bundle`) works inside it too.
#
# This is the escape hatch for the GitHub Actions outage: it needs no
# GitHub account, no runner registration and no minutes. The only
# requirement is a Docker daemon.
#
# Pass extra args to run a single gate:
#
#   docker run --rm -v "$PWD:/src" -w /src vault-ci --job rust

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:/usr/local/bun/bin:$PATH

# Tauri v2's Linux prerequisites, plus the tools the release scripts
# need (bash, python3, unzip, curl) and git for the mode check.
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential \
      curl \
      file \
      git \
      libayatana-appindicator3-dev \
      libgtk-3-dev \
      librsvg2-dev \
      libsecret-1-dev \
      libssl-dev \
      libwebkit2gtk-4.1-dev \
      patchelf \
      python3 \
      unzip \
      ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Rust (stable, matching src-tauri/rust-toolchain.toml).
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain stable \
 && rustup component add rustfmt clippy \
 && chmod -R a+w $RUSTUP_HOME $CARGO_HOME

# bun (the package manager the repo uses).
#
# BUN_INSTALL must be set explicitly. The official installer ignores
# PATH and installs to "$HOME/.bun" unless BUN_INSTALL says otherwise,
# so without it the binary lands in /root/.bun and the `chmod -R a+w
# /usr/local/bun` below fails with "No such file or directory" — which
# is how this layer broke.
ENV BUN_INSTALL=/usr/local/bun
RUN curl -fsSL https://bun.sh/install | bash \
 && chmod -R a+w /usr/local/bun \
 && ln -sf /usr/local/bun/bin/bun /usr/local/bin/bun \
 && ln -sf /usr/local/bun/bin/bun /usr/local/bin/bunx \
 && bun --version

WORKDIR /src

# Dependency layers are cached separately from the source.
COPY package.json bun.lock ./
RUN bun install --frozen-lockfile

COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./src-tauri/
RUN cd src-tauri && cargo fetch --locked

COPY . .

ENTRYPOINT ["bash", "scripts/ci.sh"]
CMD ["--quick"]
