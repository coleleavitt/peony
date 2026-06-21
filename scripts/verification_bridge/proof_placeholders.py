from __future__ import annotations

import re
import sys
from pathlib import Path


def strip_comments_and_strings(text: str) -> str:
    out: list[str] = []
    depth = 0
    in_string = False
    i = 0
    while i < len(text):
        ch = text[i]
        nxt = text[i : i + 2]
        if depth:
            if nxt == "(*":
                depth += 1
                out.append("  ")
                i += 2
            elif nxt == "*)":
                depth -= 1
                out.append("  ")
                i += 2
            else:
                out.append("\n" if ch == "\n" else " ")
                i += 1
        elif in_string:
            if ch == '"':
                if i + 1 < len(text) and text[i + 1] == '"':
                    out.append("  ")
                    i += 2
                else:
                    in_string = False
                    out.append(" ")
                    i += 1
            else:
                out.append("\n" if ch == "\n" else " ")
                i += 1
        else:
            if nxt == "(*":
                depth = 1
                out.append("  ")
                i += 2
            elif ch == '"':
                in_string = True
                out.append(" ")
                i += 1
            else:
                out.append(ch)
                i += 1
    return "".join(out)


def proof_files(proof_path: Path) -> list[Path]:
    if proof_path.is_file():
        if proof_path.suffix != ".v":
            print(f"error: proof scan file must end in .v: {proof_path}", file=sys.stderr)
            sys.exit(2)
        return [proof_path]
    if proof_path.is_dir():
        proofs = sorted(proof_path.glob("*.v"))
        if proofs:
            return proofs
        print(f"error: no .v files found in {proof_path}", file=sys.stderr)
        sys.exit(2)
    print(f"error: proof path does not exist: {proof_path}", file=sys.stderr)
    sys.exit(2)


def main() -> int:
    if len(sys.argv) != 2:
        print("error: expected proof path", file=sys.stderr)
        return 2
    vernacular_re = re.compile(
        r"^\s*(?:(?:Local|Global|Polymorphic|Monomorphic|Program)\s+)*"
        r"(Admitted|Axiom|Parameter|Conjecture)\b"
    )
    admit_re = re.compile(r"\badmit\b")
    violations: list[tuple[Path, int, str]] = []
    proofs = proof_files(Path(sys.argv[1]))

    for proof in proofs:
        cleaned = strip_comments_and_strings(proof.read_text(encoding="utf-8"))
        for line_no, line in enumerate(cleaned.splitlines(), start=1):
            match = vernacular_re.search(line)
            if match:
                violations.append((proof, line_no, match.group(1)))
            elif admit_re.search(line):
                violations.append((proof, line_no, "admit"))

    if violations:
        print("forbidden Rocq proof placeholders found:", file=sys.stderr)
        for proof, line_no, token in violations:
            print(f"{proof}:{line_no}: {token}", file=sys.stderr)
        return 1
    print(f"Proof-placeholder scan passed ({len(proofs)} .v files).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
