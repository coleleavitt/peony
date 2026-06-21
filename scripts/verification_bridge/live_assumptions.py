from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path


ASSUMPTION_STATUSES = {"theorem-bridged", "implementation-verified"}
ALLOWED_ASSUMPTIONS = {"FunctionalExtensionality.functional_extensionality_dep"}


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def normalize(text: str) -> str:
    return " ".join(text.split())


def validate_assumptions_text(label: str, text: str) -> None:
    if "Closed under the global context" in text and "Axioms:" not in text:
        return
    if "Axioms:" not in text:
        fail(f"{label}: unknown Print Assumptions output")
    names = set(re.findall(r"^([A-Za-z0-9_.]+)\s*:", text, flags=re.MULTILINE))
    names.discard("Axioms")
    unknown = sorted(name for name in names if name not in ALLOWED_ASSUMPTIONS)
    if unknown:
        fail(f"{label}: non-allowlisted assumptions: {', '.join(unknown)}")
    if not names:
        fail(f"{label}: Axioms block has no named axiom")


def coq_print_assumptions(repo_root: Path, theorem: str) -> str:
    if "." not in theorem:
        fail(f"{theorem}: theorem names used by public claims must be module-qualified")
    module, name = theorem.rsplit(".", 1)
    script = f"From Peony Require Import {module}.\nPrint Assumptions {name}.\n"
    result = subprocess.run(
        ["coqtop", "-quiet", "-Q", "rocq-tests", "Peony"],
        cwd=repo_root,
        input=script,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        fail(f"{theorem}: coqtop Print Assumptions failed: {result.stderr.strip()}")
    return result.stdout.strip()


def expected_artifacts(claims_path: Path) -> dict[str, str]:
    data = json.loads(claims_path.read_text(encoding="utf-8"))
    expected: dict[str, str] = {}
    for claim in data["claims"]:
        if claim["status"] not in ASSUMPTION_STATUSES:
            continue
        theorems = claim["theorems"]
        artifacts = claim["assumption_artifacts"]
        if len(theorems) != len(artifacts):
            fail(f"{claim['id']}: theorem-backed claims must list one assumption artifact per theorem")
        for theorem, artifact in zip(theorems, artifacts):
            prior = expected.get(theorem)
            if prior is not None and prior != artifact:
                fail(f"{theorem}: multiple assumption artifacts listed: {prior}, {artifact}")
            expected[theorem] = artifact
    return expected


def main() -> int:
    if len(sys.argv) != 3:
        print("error: expected claims file and repo root", file=sys.stderr)
        return 2
    claims_path = Path(sys.argv[1])
    repo_root = Path(sys.argv[2])
    expected = expected_artifacts(claims_path)
    for theorem, artifact in sorted(expected.items()):
        artifact_path = repo_root / artifact
        if not artifact_path.is_file():
            fail(f"{theorem}: assumption artifact missing: {artifact}")
        recorded = artifact_path.read_text(encoding="utf-8").strip()
        live = coq_print_assumptions(repo_root, theorem)
        validate_assumptions_text(theorem, live)
        if normalize(recorded) != normalize(live):
            fail(f"{theorem}: recorded assumption artifact differs from live Print Assumptions output")
    print(f"Live Print Assumptions audit passed ({len(expected)} theorem artifacts).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
