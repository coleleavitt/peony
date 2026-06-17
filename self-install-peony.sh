#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
    cat <<'USAGE'
usage: ./self-install-peony.sh [--no-install] [--install-dir DIR]

Build Peony once with the normal configured linker, then rebuild the Peony
release binary with that freshly built Peony as rustc's final linker.

By default this installs the self-linked release binary to:
  ${CARGO_HOME:-$HOME/.cargo}/bin/peony

Options:
  --no-install       Leave the self-linked binary at target/release/peony only.
  --install-dir DIR  Install peony into DIR instead of Cargo's bin directory.
  -h, --help         Show this help.

Set PEONY_USE_RUSTC_WRAPPER=1 if you intentionally want to keep RUSTC_WRAPPER.
USAGE
}

install_binary=1
install_dir="${CARGO_HOME:-"$HOME/.cargo"}/bin"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-install)
            install_binary=0
            shift
            ;;
        --install-dir)
            if [[ $# -lt 2 ]]; then
                echo "missing value for --install-dir" >&2
                exit 2
            fi
            install_dir="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$repo_root"

if [[ "${PEONY_USE_RUSTC_WRAPPER:-0}" != "1" ]]; then
    export RUSTC_WRAPPER=
fi

target_root="${CARGO_TARGET_DIR:-target}"
bootstrap_dir="$target_root/self-linked"
bootstrap="$bootstrap_dir/peony-bootstrap"
release_bin="$target_root/release/peony"

echo "==> stage 1: build bootstrap peony with the configured linker"
cargo build --release -p peony --bin peony

mkdir -p "$bootstrap_dir"
cp "$release_bin" "$bootstrap"
chmod 0755 "$bootstrap"

echo "==> stage 2: relink peony release using bootstrap peony"
cargo rustc -p peony --release --bin peony -- \
    -C "linker=$repo_root/$bootstrap" \
    -C linker-flavor=ld \
    -C link-self-contained=no

echo "==> self-linked binary: $repo_root/$release_bin"

if [[ "$install_binary" -eq 1 ]]; then
    mkdir -p "$install_dir"
    install -m 0755 "$release_bin" "$install_dir/peony"
    echo "==> installed: $install_dir/peony"
else
    echo "==> install skipped"
fi
