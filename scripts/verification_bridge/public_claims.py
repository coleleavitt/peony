from __future__ import annotations

import json
import re
import sys
from pathlib import Path


ALLOWED_STATUSES = {"model-only", "bridge-tested", "theorem-bridged", "implementation-verified"}
ASSUMPTION_STATUSES = {"theorem-bridged", "implementation-verified"}
ALLOWED_ASSUMPTIONS = {"FunctionalExtensionality.functional_extensionality_dep"}
REQUIRED_DOC_REFS = {
    "README.md": "VERIFICATION_CLAIMS.md",
    "rocq-tests/README.md": "VERIFICATION_CLAIMS.md",
    "docs/THEOREM_TO_RUST_BRIDGES.md": "VERIFICATION_CLAIMS",
}


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def required_str(value: object, field: str, claim_id: str) -> str:
    if not isinstance(value, str) or not value.strip():
        fail(f"{claim_id}: {field} must be a non-empty string")
    return value


def required_list(value: object, field: str, claim_id: str) -> list[object]:
    if not isinstance(value, list):
        fail(f"{claim_id}: {field} must be a list")
    return value


def completed_todos(path: Path) -> dict[str, bool]:
    if not path.is_file():
        fail(f"dependency plan not found: {path}")
    completed: dict[str, bool] = {}
    todo_re = re.compile(r"^\s*-\s+\[(?P<mark>[ xX])\]\s+\d+\.\s+(?P<id>[A-Z][A-Za-z0-9]*)\b")
    for line in path.read_text(encoding="utf-8").splitlines():
        if match := todo_re.match(line):
            completed[match.group("id")] = match.group("mark").lower() == "x"
    if not completed:
        fail(f"no top-level todos found in dependency plan: {path}")
    return completed


def validate_assumption_artifact(repo_root: Path, relative: str) -> None:
    path = repo_root / relative
    if not path.is_file():
        fail(f"assumption artifact missing: {relative}")
    text = path.read_text(encoding="utf-8").strip()
    if "Closed under the global context" in text and "Axioms:" not in text:
        return
    if "Axioms:" not in text:
        fail(f"assumption artifact has unknown format: {relative}")
    names = set(re.findall(r"^([A-Za-z0-9_.]+)\s*:", text, flags=re.MULTILINE))
    names.discard("Axioms")
    unknown = sorted(name for name in names if name not in ALLOWED_ASSUMPTIONS)
    if unknown:
        fail(f"assumption artifact {relative} contains non-allowlisted assumptions: {', '.join(unknown)}")


def validate_existing_evidence(repo_root: Path, relative: object, claim_id: str) -> None:
    if not isinstance(relative, str) or not relative.strip():
        fail(f"{claim_id}: evidence entries must be non-empty strings")
    if "://" in relative or relative.startswith("/"):
        fail(f"{claim_id}: evidence must be a repo-relative file path: {relative}")
    if not (repo_root / relative).is_file():
        fail(f"{claim_id}: evidence artifact missing: {relative}")


def validate_claim(claim: dict[str, object], repo_root: Path, todo_state: dict[str, bool]) -> str:
    claim_id = required_str(claim.get("id"), "id", "<unknown>")
    status = required_str(claim.get("status"), "status", claim_id)
    if status not in ALLOWED_STATUSES:
        fail(f"{claim_id}: unknown status {status!r}")
    required_str(claim.get("claim"), "claim", claim_id)
    required_str(claim.get("scope"), "scope", claim_id)
    depends = [str(item) for item in required_list(claim.get("depends_on_todos"), "depends_on_todos", claim_id)]
    theorems = [str(item) for item in required_list(claim.get("theorems"), "theorems", claim_id)]
    evidence = required_list(claim.get("evidence"), "evidence", claim_id)
    artifacts = [str(item) for item in required_list(claim.get("assumption_artifacts"), "assumption_artifacts", claim_id)]
    trusted_base = required_list(claim.get("trusted_base"), "trusted_base", claim_id)
    not_claimed = required_list(claim.get("not_claimed"), "not_claimed", claim_id)
    if status == "implementation-verified":
        incomplete = [todo for todo in depends if not todo_state.get(todo, False)]
        if incomplete:
            fail(f"{claim_id}: implementation-verified claim depends on incomplete todos: {', '.join(incomplete)}")
        if not trusted_base or not not_claimed or not evidence:
            fail(f"{claim_id}: implementation-verified claims need trusted_base, not_claimed, and evidence")
        for artifact in evidence:
            validate_existing_evidence(repo_root, artifact, claim_id)
    if status in ASSUMPTION_STATUSES:
        if not theorems or not artifacts:
            fail(f"{claim_id}: {status} claims must list theorem constants and Print Assumptions artifacts")
        for artifact in artifacts:
            validate_assumption_artifact(repo_root, artifact)
    return status


def main() -> int:
    if len(sys.argv) != 4:
        print("error: expected claims file, repo root, and plan path", file=sys.stderr)
        return 2
    claims_path = Path(sys.argv[1])
    repo_root = Path(sys.argv[2])
    plan_path = Path(sys.argv[3])
    if not claims_path.is_file():
        fail(f"claim table does not exist: {claims_path}")
    data = json.loads(claims_path.read_text(encoding="utf-8"))
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        fail("claim table schema_version must be 1")
    definitions = data.get("status_definitions")
    if not isinstance(definitions, dict) or ALLOWED_STATUSES - set(definitions):
        fail("claim table must define every allowed status")
    claims = data.get("claims")
    if not isinstance(claims, list) or not claims:
        fail("claim table must contain non-empty claims list")
    todo_state = completed_todos(plan_path)
    seen_ids: set[str] = set()
    seen_statuses: set[str] = set()
    for raw_claim in claims:
        if not isinstance(raw_claim, dict):
            fail("each claim must be an object")
        claim_id = required_str(raw_claim.get("id"), "id", "<unknown>")
        if claim_id in seen_ids:
            fail(f"duplicate claim id: {claim_id}")
        seen_ids.add(claim_id)
        seen_statuses.add(validate_claim(raw_claim, repo_root, todo_state))
    missing = sorted(ALLOWED_STATUSES - seen_statuses)
    if missing:
        fail("claim table does not distinguish statuses: " + ", ".join(missing))
    for relative, marker in REQUIRED_DOC_REFS.items():
        path = repo_root / relative
        if not path.is_file() or marker not in path.read_text(encoding="utf-8"):
            fail(f"{relative} must reference {marker}")
    print(f"Public claim table validation passed ({len(claims)} claims).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
