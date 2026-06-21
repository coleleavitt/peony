#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
HELPER_DIR="${SCRIPT_DIR}/verification_bridge"
PROOF_DIR="${REPO_ROOT}/rocq-tests"
MAPPING_FILE="${REPO_ROOT}/docs/THEOREM_TO_RUST_BRIDGES.md"
CLAIMS_FILE="${REPO_ROOT}/docs/VERIFICATION_CLAIMS.json"
WORDING_ROOT="${REPO_ROOT}"
SCAN_ONLY=0
WORDING_ONLY=0

usage() {
  cat <<'USAGE'
Usage:
  bash scripts/check-verification-bridges.sh
  bash scripts/check-verification-bridges.sh --scan-placeholders <proof-path>
  bash scripts/check-verification-bridges.sh --scan-only [proof-path]
  bash scripts/check-verification-bridges.sh --check-wording <public-doc-root>

Options:
  --scan-placeholders <proof-path>
                           Run only the Rocq proof-placeholder scan against a
                           .v file or a directory containing .v files.
  --scan-only [proof-path] Backward-compatible alias for --scan-placeholders.
                           Defaults to rocq-tests when proof-path is omitted.
  --mapping <file>         Mapping table to validate in the full gate.
  --claims <file>          Public verification-claim JSON to validate in the
                           full gate.
  --check-wording <path>   Run only the public-doc overclaim wording scan
                           against a repo root or scratch doc root.
  -h, --help               Show this help.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --scan-placeholders)
      if [[ $# -lt 2 ]]; then
        echo "error: --scan-placeholders requires a .v file or proof directory" >&2
        exit 2
      fi
      SCAN_ONLY=1
      PROOF_DIR="$2"
      shift 2
      ;;
    --scan-only)
      SCAN_ONLY=1
      shift
      if [[ $# -gt 0 && "$1" != --* ]]; then
        PROOF_DIR="$1"
        shift
      fi
      ;;
    --mapping)
      if [[ $# -lt 2 ]]; then
        echo "error: --mapping requires a file path" >&2
        exit 2
      fi
      MAPPING_FILE="$2"
      shift 2
      ;;
    --claims)
      if [[ $# -lt 2 ]]; then
        echo "error: --claims requires a file path" >&2
        exit 2
      fi
      CLAIMS_FILE="$2"
      shift 2
      ;;
    --check-wording)
      if [[ $# -lt 2 ]]; then
        echo "error: --check-wording requires a public doc root or file" >&2
        exit 2
      fi
      WORDING_ONLY=1
      WORDING_ROOT="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

cd "${REPO_ROOT}"

run_phase() {
  local label="$1"
  shift
  printf '\n==> %s\n' "${label}"
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

run_python_gate() {
  local label="$1"
  shift
  run_phase "${label}" python3 "$@"
}

scan_proof_placeholders() {
  run_python_gate "Proof-placeholder scan" \
    "${HELPER_DIR}/proof_placeholders.py" "$1"
}

validate_mapping_table() {
  run_python_gate "Theorem-to-Rust mapping validation" \
    "${HELPER_DIR}/mapping_table.py" "$1"
}

validate_public_claims() {
  run_python_gate "Public verification claim validation" \
    "${HELPER_DIR}/public_claims.py" "$1" "${REPO_ROOT}" \
    "${REPO_ROOT}/.omo/plans/peony-implementation-verification.md"
}

check_public_wording() {
  run_python_gate "Public verification wording gate" \
    "${HELPER_DIR}/public_wording.py" "$1"
}

validate_live_assumption_audit() {
  run_python_gate "Live Print Assumptions audit" \
    "${HELPER_DIR}/live_assumptions.py" "$1" "${REPO_ROOT}"
}

declare -A CACHE_BACKUP=()
CACHE_TMP=""

snapshot_rocq_caches() {
  CACHE_TMP="$(mktemp -d)"
  local cache
  for cache in rocq-tests/.lia.cache rocq-tests/.nia.cache; do
    if [[ -e "${cache}" ]]; then
      cp "${cache}" "${CACHE_TMP}/$(basename "${cache}")"
      CACHE_BACKUP["${cache}"]=1
    else
      CACHE_BACKUP["${cache}"]=0
    fi
  done
}

restore_rocq_caches() {
  if [[ -z "${CACHE_TMP}" || ! -d "${CACHE_TMP}" ]]; then
    return
  fi
  local cache base
  for cache in rocq-tests/.lia.cache rocq-tests/.nia.cache; do
    base="$(basename "${cache}")"
    if [[ "${CACHE_BACKUP[${cache}]}" == 1 ]]; then
      cp "${CACHE_TMP}/${base}" "${cache}"
    else
      rm -f "${cache}"
    fi
  done
  rm -rf "${CACHE_TMP}"
}

if [[ "${SCAN_ONLY}" -eq 1 ]]; then
  scan_proof_placeholders "${PROOF_DIR}"
  exit 0
fi

if [[ "${WORDING_ONLY}" -eq 1 ]]; then
  check_public_wording "${WORDING_ROOT}"
  exit 0
fi

snapshot_rocq_caches
trap restore_rocq_caches EXIT

scan_proof_placeholders "${PROOF_DIR}"
validate_mapping_table "${MAPPING_FILE}"
validate_public_claims "${CLAIMS_FILE}"
check_public_wording "${WORDING_ROOT}"

run_phase "Rust bridge tests: V1 verification witnesses" \
  cargo test -p peony-verification --lib
run_phase "Rust bridge tests: peony-cache partial relink" \
  cargo test -p peony-cache --test partial_relink
run_phase "Rust bridge tests: peony incremental byte identity" \
  cargo test -p peony --test incremental
run_phase "Rust bridge tests: peony-emit input_work" \
  cargo test -p peony-emit --lib input_work::tests
run_phase "Rust bridge tests: peony-reloc byte formulas" \
  cargo test -p peony-reloc --lib
run_phase "Rust bridge tests: peony-layout layout bridge" \
  cargo test -p peony-layout --test layout_bridge
run_phase "Rust bridge tests: peony-layout layout/GC bridge" \
  cargo test -p peony-layout --test layout_gc_bridge
run_phase "Rust bridge tests: peony-layout ICF bridge" \
  cargo test -p peony-layout --test icf_bridge
run_phase "Rust bridge tests: peony-symbols bridge" \
  cargo test -p peony-symbols --test symbol_bridge

run_phase "Rocq proof build" make -C rocq-tests -B
validate_live_assumption_audit "${CLAIMS_FILE}"
run_phase "Rocq kernel check" \
  coqchk -Q rocq-tests Peony \
    Peony.SymbolLattice \
    Peony.RelocDisjoint \
    Peony.RelocMonoid \
    Peony.SectionGC \
    Peony.Layout \
    Peony.IncrementalSoundness \
    Peony.IncrementalCostBound \
    Peony.ParallelSchedule \
    Peony.ICFSoundness
run_phase "Rocq proof check target" make -C rocq-tests check

printf '\nBridge verification gate passed.\n'
