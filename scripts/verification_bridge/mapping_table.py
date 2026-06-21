from __future__ import annotations

import re
import sys
from pathlib import Path


REQUIRED = [
    "SymbolLattice.sjoin",
    "RelocDisjoint.accepted_emit_ranges_parallel_batches_deterministic",
    "RelocDisjoint.apply1 / footprint",
    "RelocMonoid.oplus / act",
    "SectionGC.gc_sound",
    "Layout.layout_assign",
    "IncrementalSoundness.incremental_relink_sound",
    "IncrementalSoundness.render",
    "IncrementalCostBound.incremental_beats_fromscratch",
    "ParallelSchedule.greedy_within_2x_opt",
    "ICFSoundness.address_safe / icf_observationally_equivalent",
]
REQUIRED_COLUMNS = ["Rocq theorem", "Bridge status", "Rust surface", "Rust test", "Trusted boundary"]


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def plain(cell: str) -> str:
    cell = re.sub(r"<br\s*/?>", " ", cell, flags=re.IGNORECASE)
    cell = re.sub(r"[`*]", "", cell)
    return " ".join(cell.strip().split())


def table_rows(mapping_path: Path) -> list[dict[str, str]]:
    header: list[str] | None = None
    rows: list[dict[str, str]] = []
    for raw in mapping_path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line.startswith("|") or not line.endswith("|"):
            continue
        cells = [cell.strip() for cell in line.strip("|").split("|")]
        if cells and all(re.fullmatch(r":?-{3,}:?", cell.strip()) for cell in cells):
            continue
        if header is None:
            header = [plain(cell) for cell in cells]
            continue
        if len(cells) != len(header):
            fail(f"malformed mapping row: {raw}")
        rows.append(dict(zip(header, cells)))
    if header is None:
        fail("no Markdown table found in mapping file")
    missing = [column for column in REQUIRED_COLUMNS if column not in header]
    if missing:
        fail("mapping table missing columns: " + ", ".join(missing))
    return rows


def validate_required_fields(by_theorem: dict[str, dict[str, str]]) -> None:
    empty_markers = {"", "-", "tbd", "todo"}
    bad_rows: list[str] = []
    for theorem, row in by_theorem.items():
        if plain(row["Bridge status"]).lower() == "implementation-bridge":
            for column in ("Rust surface", "Rust test", "Trusted boundary"):
                if plain(row[column]).lower() in empty_markers:
                    bad_rows.append(f"{theorem}: empty {column}")
    for theorem in REQUIRED:
        row = by_theorem[theorem]
        for column in ("Rust test", "Trusted boundary"):
            if plain(row[column]).lower() in empty_markers:
                bad_rows.append(f"{theorem}: empty required {column}")
    if bad_rows:
        print("error: mapping rows are missing required fields:", file=sys.stderr)
        for row in bad_rows:
            print(f"  - {row}", file=sys.stderr)
        sys.exit(1)


def require_markers(row: dict[str, str], markers: list[str], label: str) -> None:
    status = plain(row["Bridge status"]).lower()
    text = plain(" ".join(row.values()))
    errors = [f"{label} row missing marker: {marker}" for marker in markers if marker not in text]
    if label == "I1" and status != "implementation-bridge":
        errors.insert(0, "ICF row must be an implementation-bridge after I1")
    if label == "N1" and status != "implementation-bridge (n1 scoped fixture)":
        errors.insert(0, "IncrementalSoundness.render row must be an N1 scoped implementation-bridge")
    if errors:
        print(f"error: {label} mapping validation failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        sys.exit(1)


def validate_source_markers(repo_root: Path) -> None:
    markers = {
        "peony-emit/src/lib.rs": ["EmitWriteReport", "emit_partial_objects_with_report"],
        "peony-verification/src/incremental.rs": ["partial_emit_writes_from_report"],
        "peony-verification/src/tests/n1_real_emit.rs": [
            "emit_partial_objects_with_report",
            "partial_emit_writes_from_report",
        ],
    }
    errors: list[str] = []
    for relative, required in markers.items():
        source_path = repo_root / relative
        if not source_path.is_file():
            errors.append(f"N1 source marker file missing: {relative}")
            continue
        source_text = source_path.read_text(encoding="utf-8")
        errors.extend(f"N1 source marker missing in {relative}: {marker}" for marker in required if marker not in source_text)
    if errors:
        print("error: N1 mapping validation failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        sys.exit(1)


def main() -> int:
    if len(sys.argv) != 2:
        print("error: expected mapping file", file=sys.stderr)
        return 2
    mapping_path = Path(sys.argv[1])
    if not mapping_path.is_file():
        fail(f"mapping table does not exist: {mapping_path}")
    mapping_text = mapping_path.read_text(encoding="utf-8")
    rows = table_rows(mapping_path)
    by_theorem = {plain(row["Rocq theorem"]): row for row in rows}
    missing = [theorem for theorem in REQUIRED if theorem not in by_theorem]
    if missing:
        fail("mapping table missing required rows: " + ", ".join(missing))
    validate_required_fields(by_theorem)
    require_markers(by_theorem["ICFSoundness.address_safe / icf_observationally_equivalent"], [
        "IcfFoldWitness", "IcfSectionWitness", "IcfFoldKeyWitness",
        "check_icf_fold_witnesses", "cargo test -p peony-layout --test icf_bridge",
        "cargo test -p peony-verification --lib",
    ], "I1")
    require_markers(by_theorem["IncrementalSoundness.render"], [
        "EmitWriteReport", "emit_partial_objects_with_report", "PartialEmitPreservationWitness",
        "PartialEmitWriteWitness", "check_partial_emit_preservation", "partial_emit_writes_from_report",
        "cargo test -p peony-cache --test partial_relink", "cargo test -p peony-verification --lib n1",
        "cargo test -p peony --test incremental",
    ], "N1")
    validate_source_markers(Path.cwd())
    if "P1 Proof-Assumption Audit Hook" not in mapping_text:
        fail("mapping table must include the P1 proof-assumption audit hook")
    print(f"Mapping validation passed ({len(rows)} rows; {len(REQUIRED)} required).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
