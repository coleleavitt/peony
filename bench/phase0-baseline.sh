#!/usr/bin/env bash
# phase0-baseline.sh — the measurement gate for the parallel/zero-copy rewrite.
#
# Produces, for every corpus, replayed through the *real* `cc -B<dir>` driver
# (crt + libc, exactly like bench.sh):
#   1. peony output sha256        — the byte-compare baseline every later phase
#                                    must match (correctness gate).
#   2. a determinism check        — link twice, the two sha256s must be equal
#                                    (catches races introduced by parallelism).
#   3. peony --stats per-phase     — self-time table at --threads 1 and
#      self-time at 1 vs N threads   --threads 0 (all cores), so we can compute
#                                    the per-phase scaling factor and re-rank
#                                    the remaining phases by measured cost
#                                    (Amdahl: biggest serial fraction wins).
#
# Usage: bench/phase0-baseline.sh [--label NAME] [--corpus NAME]
#   --label  tag for the output dir under bench/baselines/ (default: git short sha)
#   --corpus restrict to one corpus (default: all)
#
# Re-run after each phase with a fresh --label to diff self-time + confirm the
# sha256s still match the phase0 baseline.
set -Eeuo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/.." && pwd)"
corp="$repo/bench/corpora"
peony_bin="$repo/target/release/peony"

label="$(git -C "$repo" rev-parse --short HEAD 2>/dev/null || echo nogit)"
want_corpus=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --label)  label="$2"; shift 2 ;;
    --corpus) want_corpus="$2"; shift 2 ;;
    -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

[ -x "$peony_bin" ] || { echo "building peony release..."; (cd "$repo" && cargo build --release -q -p peony); }
command -v "${CC:-cc}" >/dev/null || { echo "no cc driver"; exit 1; }
drv_default=cc

outdir="$repo/bench/baselines/$label"
mkdir -p "$outdir"
shm="$(mktemp -d /dev/shm/peony-p0.XXXXXX 2>/dev/null || mktemp -d)"
trap 'rm -rf "$shm"' EXIT

# A -B dir that exposes peony as `ld` so the cc driver calls it.
bdir="$shm/B-peony"; mkdir -p "$bdir"; ln -sf "$peony_bin" "$bdir/ld"

ncpu="$(nproc)"
echo "host: $(uname -srm)  cores: $ncpu  label: $label"
echo "peony: $($peony_bin --version 2>/dev/null || echo '?')"
summary="$outdir/SUMMARY.md"
: > "$summary"
echo "# Phase-0 baseline — label \`$label\`" >> "$summary"
echo "" >> "$summary"
echo "host: \`$(uname -srm)\`  cores: $ncpu  date: $(date -Is)" >> "$summary"
echo "" >> "$summary"

run_corpus() {
  local cdir="$1" cname; cname="$(basename "$cdir")"
  [ -f "$cdir/link.args" ] || { echo "  ($cname: no link.args, skip)"; return; }
  local drv; drv="$(cat "$cdir/driver" 2>/dev/null || echo "$drv_default")"
  echo "════ $cname ($(jq -r '.n_inputs' "$cdir/meta.json" 2>/dev/null || echo '?') inputs, driver=$drv) ════"

  mapfile -t ARGS < <(cat "$cdir/link.args")
  local A=(); local a
  for a in "${ARGS[@]}"; do
    case "$a" in inputs/*) A+=("$cdir/$a") ;; *) A+=("$a") ;; esac
  done

  # --- determinism + sha256 baseline (threads=0, two independent links) -------
  local o1="$shm/$cname.1" o2="$shm/$cname.2"
  "$drv" -B"$bdir" -Wl,--threads=0 "${A[@]}" -o "$o1" 2>"$shm/$cname.link1.err" || {
    echo "  LINK FAILED — see $shm/$cname.link1.err"; sed -n '1,5p' "$shm/$cname.link1.err"; return; }
  "$drv" -B"$bdir" -Wl,--threads=0 "${A[@]}" -o "$o2" 2>/dev/null || true
  local s1 s2; s1="$(sha256sum "$o1" | cut -d' ' -f1)"; s2="$(sha256sum "$o2" | cut -d' ' -f1)"
  local det="DETERMINISTIC"; [ "$s1" = "$s2" ] || det="⚠ NONDETERMINISTIC ($s1 vs $s2)"
  echo "$s1  $cname" >> "$outdir/sha256.txt"
  echo "  sha256=$s1  $det"

  # --- per-phase self-time at 1 thread and N threads --------------------------
  # peony prints the --stats table to stderr; capture both runs.
  "$drv" -B"$bdir" -Wl,--stats -Wl,--threads=1 "${A[@]}" -o "$shm/$cname.t1" \
      2>"$outdir/$cname.stats.t1.txt" || true
  "$drv" -B"$bdir" -Wl,--stats -Wl,--threads=0 "${A[@]}" -o "$shm/$cname.tN" \
      2>"$outdir/$cname.stats.tN.txt" || true

  # --- emit a compact markdown block ------------------------------------------
  {
    echo "## $cname"
    echo ""
    echo "- inputs: $(jq -r '.n_inputs' "$cdir/meta.json" 2>/dev/null || echo '?')"
    echo "- sha256: \`$s1\` — $det"
    echo ""
    echo "### --stats @ threads=1"
    echo '```'
    grep -A40 -iE 'phase|====|total' "$outdir/$cname.stats.t1.txt" 2>/dev/null | head -40 || echo "(no stats)"
    echo '```'
    echo "### --stats @ threads=$ncpu (0=all)"
    echo '```'
    grep -A40 -iE 'phase|====|total' "$outdir/$cname.stats.tN.txt" 2>/dev/null | head -40 || echo "(no stats)"
    echo '```'
    echo ""
  } >> "$summary"
}

if [ -n "$want_corpus" ]; then
  run_corpus "$corp/$want_corpus"
else
  for cdir in "$corp"/*/; do [ -f "$cdir/link.args" ] || continue; run_corpus "$cdir"; done
fi

echo ""
echo "baseline written to: $outdir"
echo "  sha256.txt          — byte-compare baseline for later phases"
echo "  SUMMARY.md          — per-phase self-time (1 vs N threads)"
echo "  <corpus>.stats.*.txt — raw --stats dumps"
