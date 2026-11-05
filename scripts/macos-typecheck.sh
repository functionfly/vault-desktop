#!/usr/bin/env bash
# macOS compile gate — type-checks the macOS build on a Linux host.
#
# WHY THIS EXISTS
# ---------------
# The app ships for macOS, but the release workflow's `macos-latest`
# job cannot start while the organisation's GitHub billing is
# unsettled. Without a gate, a macOS-only compile error would sit
# unnoticed until a macOS runner finally exists.
#
# This is a COMPILE GATE, NOT A BUILD. It runs `cargo check` for
# `aarch64-apple-darwin`, which type-checks every `cfg(target_os =
# "macos")` branch in the crate WITHOUT LINKING. It cannot produce a
# `.app`, a `.dmg`, or a runnable binary — that needs a real macOS
# host with the Apple SDK, the Tauri macOS bundler, and an Apple
# Developer ID certificate.
#
# HOW IT CROSS-COMPILES
# ----------------------
# Three things stand between a Linux box and a macOS `cargo check`.
#
#   1. The Apple SDK is absent — but only the HEADERS are needed to
#      compile, and Zig bundles the macOS libc headers and emits
#      Mach-O arm64 objects. `zig cc` is the compiler.
#
#   2. cc-rs speaks APPLE's compiler dialect, not Zig's:
#          -arch arm64                    (Apple; Zig does not know it)
#          --target=arm64-apple-macosx    (Apple's triple spelling)
#          -mmacosx-version-min=11.0      (Zig does know this one)
#      A small wrapper rewrites the first two and passes the rest
#      through. See `zigcc` below.
#
#   3. CMake — which aws-lc-sys uses — demands `install_name_tool` as
#      soon as `CMAKE_SYSTEM_NAME=Darwin`. LLVM ships the same tool as
#      `llvm-install-name-tool`.
#
# PREREQUISITES
# -------------
#   - rustup target add aarch64-apple-darwin   (this script does it)
#   - zig        https://ziglang.org/download/   (or set ZIG_BIN)
#   - LLVM 17+   for llvm-install-name-tool     (or set INSTALL_NAME_TOOL)
#   - cmake      aws-lc-sys needs it
#
# USAGE
# -----
#   ./scripts/macos-typecheck.sh            # check
#   ./scripts/macos-typecheck.sh --clean    # wipe the darwin target first
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

TARGET=aarch64-apple-darwin
ZIG_TARGET="${ZIG_TARGET:-aarch64-macos}"
CLEAN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --clean) CLEAN=1; shift ;;
    -h|--help) sed -n '2,60p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 64 ;;
  esac
done

red() { printf '\033[31m%s\033[0m\n' "$*"; }
grn() { printf '\033[32m%s\033[0m\n' "$*"; }
fail() { red "macOS type-check: $1"; exit 1; }

# ── Locate the tools ───────────────────────────────────────────────────
ZIG_BIN="${ZIG_BIN:-$(command -v zig || true)}"
if [[ -z "$ZIG_BIN" ]]; then
  for c in "$HOME/.local/zig/zig" /opt/zig/zig /usr/local/bin/zig; do
    [[ -x "$c" ]] && ZIG_BIN="$c" && break
  done
fi
[[ -n "$ZIG_BIN" ]] || fail "zig not found. Install it from https://ziglang.org/download/ or set ZIG_BIN."

INSTALL_NAME_TOOL="${INSTALL_NAME_TOOL:-}"
if [[ -z "$INSTALL_NAME_TOOL" ]]; then
  # Newest first: llvm-14's is too old for some Mach-O features.
  for v in 21 20 19 18 17 16 15; do
    if [[ -x "/usr/lib/llvm-$v/bin/llvm-install-name-tool" ]]; then
      INSTALL_NAME_TOOL="/usr/lib/llvm-$v/bin/llvm-install-name-tool"
      break
    fi
  done
fi
[[ -z "$INSTALL_NAME_TOOL" ]] && INSTALL_NAME_TOOL="$(command -v llvm-install-name-tool || true)"
[[ -n "$INSTALL_NAME_TOOL" ]] || fail "llvm-install-name-tool not found. Install LLVM 17+ or set INSTALL_NAME_TOOL."

command -v cargo >/dev/null 2>&1 || fail "cargo not found."
command -v cmake >/dev/null 2>&1 || fail "cmake not found (aws-lc-sys needs it)."

echo "=== macOS compile gate ==="
echo "  target:        $TARGET"
echo "  zig:           $ZIG_BIN ($("$ZIG_BIN" version 2>/dev/null))"
echo "  install_name:  $INSTALL_NAME_TOOL"

# ── The Rust target ───────────────────────────────────────────────────
if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
  echo "  installing the Rust target..."
  rustup target add "$TARGET" || fail "could not install $TARGET"
fi

# ── The compiler wrappers ─────────────────────────────────────────────
# cargo passes a compiler as a single executable path, so the
# "zig cc" two-word form needs a shell wrapper. The wrapper also
# translates Apple's flag dialect into Zig's.
BIN_DIR="${TMPDIR:-/tmp}/vault-macos-toolchain"
mkdir -p "$BIN_DIR"

cat > "$BIN_DIR/zigcc" <<EOF
#!/usr/bin/env bash
# Translates Apple's compiler dialect into Zig's:
#   -arch <x>                   -> dropped (Zig uses -target)
#   --target=arm64-apple-macosx -> --target=$ZIG_TARGET
#   -mmacosx-version-min=...    -> passed through (Zig understands it)
ZIG="$ZIG_BIN"
ZIG_TARGET="$ZIG_TARGET"
args=(); target_seen=0; skip_next=0
for a in "\$@"; do
  if [[ \$skip_next -eq 1 ]]; then skip_next=0; continue; fi
  case "\$a" in
    -arch)      skip_next=1 ;;
    -arch=*)    ;;
    --target=*) args+=("--target=\$ZIG_TARGET"); target_seen=1 ;;
    -target)    args+=("-target"); skip_next=1; args+=("\$ZIG_TARGET"); target_seen=1 ;;
    *)          args+=("\$a") ;;
  esac
done
[[ \$target_seen -eq 0 ]] && args=("--target=\$ZIG_TARGET" "\${args[@]}")
exec "\$ZIG" cc "\${args[@]}"
EOF

for pair in "zigxx:c++" "zigar:ar" "zigranlib:ranlib"; do
  name="${pair%%:*}"; sub="${pair#*:}"
  cat > "$BIN_DIR/$name" <<EOF
#!/usr/bin/env bash
exec "$ZIG_BIN" $sub "\$@"
EOF
done

# CMake looks for install_name_tool by that exact name, on PATH.
cat > "$BIN_DIR/install_name_tool" <<EOF
#!/usr/bin/env bash
exec "$INSTALL_NAME_TOOL" "\$@"
EOF

chmod +x "$BIN_DIR"/zig* "$BIN_DIR/install_name_tool"

export CC_aarch64_apple_darwin="$BIN_DIR/zigcc"
export CXX_aarch64_apple_darwin="$BIN_DIR/zigxx"
export AR_aarch64_apple_darwin="$BIN_DIR/zigar"
export RANLIB_aarch64_apple_darwin="$BIN_DIR/zigranlib"
export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$BIN_DIR/zigcc"
# aws-lc-sys's assembly needs the Apple assembler; the pure-C fallback
# is the same code without the NEON/SHA3 fast paths. It is the same
# crypto, just not the accelerated path — and it is what makes the
# cross-build possible without an Apple toolchain.
export AWS_LC_SYS_NO_ASM=1
export PATH="$BIN_DIR:$PATH"

if [[ "$CLEAN" -eq 1 ]]; then
  echo "  cleaning the $TARGET target directory..."
  rm -rf "src-tauri/target/$TARGET"
fi

# ── The check ─────────────────────────────────────────────────────────
echo ""
echo "=== cargo check --target $TARGET ==="
OUT=$(cd src-tauri && cargo check --target "$TARGET" 2>&1)
RC=$?

echo "$OUT" | grep -E "^(error|warning: unused)" | head -20 | sed 's/^/  /'
echo "$OUT" | grep -E "Checking functionfly|Finished" | tail -3 | sed 's/^/  /'

if [[ $RC -ne 0 ]] || echo "$OUT" | grep -qE "^error"; then
  fail "the macOS build does not type-check"
fi

grn "macOS type-check: PASS"
echo "  (a compile gate, not a build — no .app or .dmg is produced)"
