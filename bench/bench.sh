#!/usr/bin/env bash
# bench.sh — rigorous, reproducible linker benchmark: peony vs mold vs lld vs ld.
#
# Methodology (see bench/BENCHMARKING.md for the why):
#   1. CORRECTNESS GATE FIRST. Every linker links the corpus; the output binary
#      must run with the corpus' REFERENCE.run argv and produce identical
#      stdout/exit to the reference (lld) link.
#      A linker that fails the gate is EXCLUDED from timing — a fast wrong linker
#      is worthless.
#   2. Isolate the link step. We replay a frozen `cc @link.args` (see capture.sh);
#      nothing is recompiled. Output goes to tmpfs (/dev/shm) to measure the
#      linker, not the SSD.
#   3. hyperfine with --warmup (warm page cache) and --runs, reporting median.
#   4. Optional CPU pinning (taskset/numactl) + governor hint for low variance.
#   5. Peak RSS measured separately via /usr/bin/time -v (hyperfine is wall-only).
#   6. Flags are held byte-identical across linkers (only the linker swaps), so
#      the comparison is apples-to-apples.
#
# Usage:
#   bench/bench.sh [--runs N] [--warmup N] [--pin "0-7"] [--threads N]
#                  [--only peony,mold,lld,bfd] [--corpus NAME] [--out DIR]
#                  [--strict-env]
set -Eeuo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/.." && pwd)"
corp="$repo/bench/corpora"
peony_bin="$repo/target/release/peony"

runs=15 warmup=3 pin="" threads="" only="" want_corpus="" outdir="$repo/bench/results" strict_env=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs)    runs="$2"; shift 2 ;;
    --warmup)  warmup="$2"; shift 2 ;;
    --pin)     pin="$2"; shift 2 ;;        # e.g. "0-7"  -> taskset -c 0-7
    --threads) threads="$2"; shift 2 ;;    # normalize linker thread count
    --only)    only="$2"; shift 2 ;;       # comma list: peony,mold,lld,bfd
    --corpus)  want_corpus="$2"; shift 2 ;;
    --out)     outdir="$2"; shift 2 ;;
    --strict-env) strict_env=1; shift ;;
    -h|--help) sed -n '2,21p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

command -v hyperfine >/dev/null || { echo "hyperfine not found (cargo install hyperfine)"; exit 1; }
[ -x "$peony_bin" ] || { echo "building peony release..."; (cd "$repo" && cargo build --release -q -p peony); }
mkdir -p "$outdir"
# Link outputs go to tmpfs (fast, measures the linker not the SSD). But /dev/shm
# is frequently mounted noexec, so the correctness gate copies binaries to an
# exec-capable dir before running them.
shm="$(mktemp -d /dev/shm/peony-bench.XXXXXX 2>/dev/null || mktemp -d)"
execdir="$(mktemp -d "${TMPDIR:-/tmp}/peony-bench-exec.XXXXXX")"
trap 'rm -rf "$shm" "$execdir"' EXIT

# ---- Build per-linker -B dirs (each exposes the chosen linker as `ld`) --------
declare -A LD_PATH=( [peony]="$peony_bin" [mold]="$(command -v mold || true)"
                     [lld]="$(command -v ld.lld || true)" [bfd]="$(command -v ld.bfd || command -v ld || true)" )
declare -A BDIR=()
all_linkers=(peony mold lld bfd)
[ -n "$only" ] && IFS=, read -r -a all_linkers <<< "$only"
linkers=()
for L in "${all_linkers[@]}"; do
  p="${LD_PATH[$L]:-}"; [ -n "$p" ] && [ -x "$p" ] || { echo "skip $L (not installed)"; continue; }
  d="$shm/B-$L"; mkdir -p "$d"; ln -sf "$p" "$d/ld"; BDIR[$L]="$d"; linkers+=("$L")
done
[ ${#linkers[@]} -ge 1 ] || { echo "no linkers available"; exit 1; }

# Per-linker thread flag (best effort; flags differ across linkers).
thread_flag() { # $1=linker
  [ -n "$threads" ] || return 0
  case "$1" in
    peony) printf -- '-Wl,--threads=%s' "$threads" ;;
    mold)  printf -- '-Wl,--threads=%s' "$threads" ;;
    lld)   printf -- '-Wl,--threads=%s' "$threads" ;;
    bfd)   : ;;  # GNU ld is single-threaded
  esac
}

load_run_args() { # $1=corpus dir
  local cdir="$1"
  RUN_ARGS=()
  [ -f "$cdir/REFERENCE.run" ] || return 0
  local line
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      ""|\#*) continue ;;
      inputs/*) RUN_ARGS+=("$cdir/$line") ;;
      *) RUN_ARGS+=("$line") ;;
    esac
  done < "$cdir/REFERENCE.run"
}

governor_note() {
  local g; g="$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo '?')"
  echo "$g"
}

# ---- Run one corpus -----------------------------------------------------------
run_corpus() {
  local cdir="$1" cname; cname="$(basename "$cdir")"
  [ -f "$cdir/link.args" ] || { echo "  ($cname has no link.args — skip)"; return; }
  local drv; drv="$(cat "$cdir/driver" 2>/dev/null || echo cc)"  # cc or c++
  echo "════════════════════════════════════════════════════════════════════"
  echo "corpus: $cname   ($(jq -r '.n_inputs' "$cdir/meta.json" 2>/dev/null || echo '?') inputs, driver=$drv)"
  mapfile -t ARGS < <(cat "$cdir/link.args")
  # Resolve relative input paths against the corpus dir.
  local A=(); local a
  for a in "${ARGS[@]}"; do
    case "$a" in inputs/*) A+=("$cdir/$a") ;; *) A+=("$a") ;; esac
  done
  local RUN_ARGS=()
  load_run_args "$cdir"

  # --- Correctness gate: link with each linker, run, compare to lld reference --
  echo "── correctness gate ──"
  if [ ${#RUN_ARGS[@]} -gt 0 ]; then
    printf "  run argv:"
    printf " %q" "${RUN_ARGS[@]}"
    printf "\n"
  fi
  local ref_out="" ref_rc="" gate_ok=()
  for L in "${linkers[@]}"; do
    local bin="$shm/out.$cname.$L"; local tf; tf="$(thread_flag "$L")"
    local out rc
    set +e
    "$drv" -B"${BDIR[$L]}" ${tf:+$tf} "${A[@]}" -o "$bin" 2>"$shm/link.$L.err"
    local link_rc=$?
    if [ "$link_rc" = 0 ]; then
      # /dev/shm is often noexec; copy to an exec-capable dir to actually run.
      local rbin="$execdir/out.$cname.$L"; cp -f "$bin" "$rbin"; chmod +x "$rbin"
      out="$("$rbin" "${RUN_ARGS[@]}" 2>/dev/null)"; rc=$?
      set -e
      printf "  %-6s link=ok run_rc=%s\n" "$L" "$rc"
      gate_ok+=("$L"); eval "G_${L}_out=\$out"; eval "G_${L}_rc=\$rc"
    else
      set -e
      printf "  %-6s link=FAIL (%s)\n" "$L" "$(grep -iE 'error|undefined|cannot|unsupported' "$shm/link.$L.err" | head -1)"
    fi
  done
  # Reference = lld if present else first gated linker.
  local ref=lld; printf '%s\n' "${gate_ok[@]}" | grep -qx lld || ref="${gate_ok[0]:-}"
  [ -n "$ref" ] || { echo "  no linker passed the gate — skipping timing"; return; }
  local rout rrc; rout="$(eval echo \"\$G_${ref}_out\")"; rrc="$(eval echo \"\$G_${ref}_rc\")"
  local timed=()
  for L in "${gate_ok[@]}"; do
    local o r; o="$(eval echo \"\$G_${L}_out\")"; r="$(eval echo \"\$G_${L}_rc\")"
    if [ "$o" = "$rout" ] && [ "$r" = "$rrc" ]; then timed+=("$L")
    else printf "  ⚠ %-6s output DIFFERS from %s reference — excluded from timing\n" "$L" "$ref"; fi
  done
  echo "  reference=$ref  timed: ${timed[*]}"
  [ ${#timed[@]} -ge 1 ] || { echo "  nothing to time"; return; }

  # --- Peak RSS (separate pass, /usr/bin/time -v) ------------------------------
  echo "── peak RSS (KB) ──"
  for L in "${timed[@]}"; do
    local tf; tf="$(thread_flag "$L")"
    local kb; kb="$(/usr/bin/time -v "$drv" -B"${BDIR[$L]}" ${tf:+$tf} "${A[@]}" -o "$shm/rss.$L" 2>&1 \
                    | awk '/Maximum resident set size/{print $NF}')"
    printf "  %-6s %s\n" "$L" "${kb:-?}"
  done

  # --- Wall-clock via hyperfine ------------------------------------------------
  echo "── wall-clock (hyperfine --warmup $warmup --runs $runs) ──"
  local pre=""; [ -n "$pin" ] && pre="taskset -c $pin "
  local args=(hyperfine --warmup "$warmup" --runs "$runs"
              --export-json "$outdir/$cname.json" --export-markdown "$outdir/$cname.md")
  for L in "${timed[@]}"; do
    local tf; tf="$(thread_flag "$L")"
    args+=(--command-name "$L" "${pre}$drv -B${BDIR[$L]} ${tf:+$tf }${A[*]} -o $shm/hf.$L")
  done
  "${args[@]}" || echo "  (hyperfine returned nonzero)"
}

gov="$(governor_note)"
echo "host: $(uname -srm)   cores: $(nproc)   governor: $gov"
if [ "$gov" != performance ]; then
  if [ "$strict_env" = 1 ]; then
    echo "fatal: --strict-env requires CPU governor 'performance' (current: $gov). See BENCHMARKING.md." >&2
    exit 3
  fi
  echo "⚠ governor is not 'performance' — numbers will be noisier. See BENCHMARKING.md."
fi
[ "$strict_env" = 0 ] || [ -n "$pin" ] || echo "⚠ --strict-env without --pin still allows scheduler migration noise." >&2
echo "linkers:"; for L in "${linkers[@]}"; do echo "  $L -> ${LD_PATH[$L]}"; done

shopt -s nullglob
if [ -n "$want_corpus" ]; then
  run_corpus "$corp/$want_corpus"
else
  found=0
  for cdir in "$corp"/*/; do [ -f "$cdir/link.args" ] || continue; found=1; run_corpus "$cdir"; done
  [ "$found" = 1 ] || echo "no corpora found. Create one with bench/capture.sh (see BENCHMARKING.md)."
fi
echo "results written under $outdir/"
