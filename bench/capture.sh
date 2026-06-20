#!/usr/bin/env bash
# capture.sh — freeze the *exact* final link of a real Rust or C/C++ build into a
# self-contained, replayable corpus directory under bench/corpora/<name>/.
#
# The corpus records the `cc`-level argv that the compiler driver used for the
# final link, with the inputs (.o/.rlib/.a) copied in and rewritten to relative
# paths, and the linker-selection flags (-fuse-ld=, -B.../gcc-ld) stripped so the
# benchmark harness can substitute any linker via `cc -B<dir>`. This isolates the
# *link step* — replaying never recompiles — which is the only honest way to time
# a linker (per BENCHMARKING.md).
#
# Usage:
#   bench/capture.sh rust-bin   <name>  <cargo-project-dir> [--release]
#   bench/capture.sh rust-crate <name>  <crate-name>        [--release]   # cargo new + add dep
#   bench/capture.sh c          <name>  <main.c> [extra cc args...]
#   bench/capture.sh cxx        <name>  <main.cpp> [extra c++ args...]
#
# Result: bench/corpora/<name>/{link.args, inputs/, meta.json, REFERENCE.run}
# `REFERENCE.run` is one runtime argument per line. Blank lines and `#` comments
# are ignored by bench.sh; edit it after capture for corpus-specific checks.
set -Eeuo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/.." && pwd)"
corp="$repo/bench/corpora"
mode="${1:?usage: capture.sh <rust-bin|rust-crate|c|cxx> <name> ...}"; shift
name="${1:?missing corpus <name>}"; shift
dest="$corp/$name"

# A logging `cc` shim that records argv+cwd then execs the real cc.
shim="$(mktemp -d)/capture-cc.sh"
log="$(mktemp)"
cat > "$shim" <<EOF
#!/usr/bin/env bash
{ echo "CWD=\$PWD"; printf '%s\n' "\$@"; } > "$log"
exec /usr/bin/cc "\$@"
EOF
chmod +x "$shim"

build_rust() {
    local proj="$1" extra="$2"
    ( cd "$proj"
      # save-temps keeps the .rcgu.o inputs alive after the link (rustc deletes
      # them by default); the cc shim captures the real link argv.
      RUSTFLAGS="-C linker=$shim -C save-temps" cargo build $extra 2>&1 | tail -2 )
}

case "$mode" in
  rust-bin)
    proj="${1:?missing cargo project dir}"; shift
    extra="${1:-}"; [ "${1:-}" = "--release" ] && extra="--release"
    build_rust "$proj" "$extra"
    ;;
  rust-crate)
    crate="${1:?missing crate name}"; shift
    extra="${1:-}"; [ "${1:-}" = "--release" ] && extra="--release"
    tmp="$(mktemp -d)/proj"
    cargo new --bin "$tmp" -q >/dev/null 2>&1 || cargo new --bin "$tmp"
    ( cd "$tmp" && cargo add "$crate" -q 2>/dev/null || true )
    # use the crate from main so it actually links in.
    cat > "$tmp/src/main.rs" <<RS
fn main() { println!("corpus link probe for $crate"); }
RS
    build_rust "$tmp" "$extra"
    ;;
  c|cxx)
    src="${1:?missing source}"; shift
    drv=/usr/bin/cc; [ "$mode" = cxx ] && drv=/usr/bin/c++
    obj="$(mktemp).o"
    "$drv" -c "$src" -o "$obj" "$@"
    # Link through the shim to capture the full driver argv.
    CC="$shim"; [ "$mode" = cxx ] && { cat > "$shim" <<EOF
#!/usr/bin/env bash
{ echo "CWD=\$PWD"; printf '%s\n' "\$@"; } > "$log"
exec /usr/bin/c++ "\$@"
EOF
chmod +x "$shim"; }
    "$shim" "$obj" -o "$(mktemp)" "$@"
    ;;
  *) echo "unknown mode: $mode" >&2; exit 2 ;;
esac

[ -s "$log" ] || { echo "capture failed: no link argv recorded" >&2; exit 1; }

# ---- Freeze: copy inputs, rewrite to relative paths, strip linker-selection ----
rm -rf "$dest"; mkdir -p "$dest/inputs"
cwd="$(sed -n '1s/^CWD=//p' "$log")"
mapfile -t raw < <(tail -n +2 "$log")

declare -a out=()
i=0
drop_next=0
for a in "${raw[@]}"; do
  if [ "$drop_next" = 1 ]; then drop_next=0; continue; fi   # skip the -o target / -plugin path
  case "$a" in
    -fuse-ld=*)        continue ;;             # let -B pick the linker
    -B*gcc-ld*)        continue ;;             # rustc's bundled lld dir
    -plugin)           drop_next=1; continue ;; # LTO plugin + its path arg: the
    -plugin-opt=*)     continue ;;             #   corpus objects are plain ELF,
                                               #   not LTO IR, so the plugin is a
                                               #   no-op that only mold/lld parse
                                               #   differently. Strip for an
                                               #   apples-to-apples link measure.
    -o)                drop_next=1; continue ;; # harness supplies its own -o
    -o*)               continue ;;             # joined -o<file> form
    *.o|*.rlib|*.a)
      # resolve relative to the build cwd, copy in, reference relatively
      p="$a"; [[ "$p" = /* ]] || p="$cwd/$p"
      if [ -f "$p" ]; then
        bn="$(printf '%05d_%s' "$i" "$(basename "$a")")"
        cp -f "$p" "$dest/inputs/$bn"
        out+=("inputs/$bn"); i=$((i+1)); continue
      fi
      out+=("$a") ;;                            # keep system libs as-is
    *) out+=("$a") ;;
  esac
done

printf '%s\n' "${out[@]}" > "$dest/link.args"
cat > "$dest/REFERENCE.run" <<'EOF'
# Runtime arguments for the benchmark correctness gate, one argv item per line.
# Empty/comment-only means "run the linked binary with no arguments".
EOF
# The compiler driver used for the final link; the harness must replay through
# the same one so driver-injected libs (-lstdc++, crt objects) match.
case "$mode" in cxx) echo c++ ;; *) echo cc ;; esac > "$dest/driver"
ninputs="$(ls "$dest/inputs" | wc -l)"
isize="$(du -sb "$dest/inputs" | cut -f1)"
cat > "$dest/meta.json" <<EOF
{
  "name": "$name",
  "mode": "$mode",
  "driver": "$(cat "$dest/driver")",
  "captured": "$(date -Is)",
  "n_inputs": $ninputs,
  "inputs_bytes": $isize,
  "host": "$(uname -srm)"
}
EOF
echo "froze corpus '$name': $ninputs inputs, $(numfmt --to=iec "$isize" 2>/dev/null || echo "$isize")B -> $dest"
