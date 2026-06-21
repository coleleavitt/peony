from __future__ import annotations

import re
import sys
from pathlib import Path
from typing import Final


CLAIM_TARGET: Final = r"(?:the\s+)?(?:rust\s+)?(?:linker|implementation)"
CLAIM_POSSESSIVE: Final = r"(?:the\s+)?(?:rust\s+)?(?:linker|implementation)'s"
CLAIM_CORRECTNESS: Final = (
    rf"(?:{CLAIM_TARGET}\s+correctness|"
    rf"{CLAIM_POSSESSIVE}\s+correctness|"
    rf"(?:the\s+)?correctness\s+of\s+{CLAIM_TARGET})"
)
END_TO_END: Final = r"end[-\s]+to[-\s]+end"
PROOF_ACTION: Final = r"(?:(?:has\s+)?(?:formally\s+)?(?:proved|proven)|(?:formally\s+)?proves?)"
VERIFY_ACTION: Final = r"(?:(?:has\s+)?(?:formally\s+)?verified|(?:formally\s+)?verifies)"


FORBIDDEN = [
    re.compile(r"\b(?:formally\s+)?verified\s+(?:rust\s+)?linker\b", re.I),
    re.compile(r"\b(?:the\s+)?(?:rust\s+)?linker\s+is\s+(?:formally\s+)?verified\b", re.I),
    re.compile(r"\b(?:proven|proved)[-\s]+correct\s+(?:rust\s+)?(?:linker|implementation)\b", re.I),
    re.compile(
        r"\b(?:the\s+)?(?:rust\s+)?(?:linker|implementation)\s+"
        r"(?:is|has\s+been|was)\s+(?:formally\s+)?(?:proven|proved)[-\s]+correct\b",
        re.I,
    ),
    re.compile(
        rf"\b{CLAIM_TARGET}\s+(?:is|has\s+been|was)\s+"
        r"(?:formally\s+)?(?:proven|proved)\b",
        re.I,
    ),
    re.compile(
        r"\b(?:the\s+)?(?:rust\s+)?(?:linker|implementation)\s+correctness\s+"
        r"(?:is|has\s+been|was)\s+(?:formally\s+)?(?:proven|proved)\b",
        re.I,
    ),
    re.compile(
        r"\b(?:the\s+)?correctness\s+of\s+(?:the\s+)?(?:rust\s+)?"
        r"(?:linker|implementation)\s+(?:is|has\s+been|was)\s+"
        r"(?:formally\s+)?(?:proven|proved)\b",
        re.I,
    ),
    re.compile(
        rf"\b{CLAIM_CORRECTNESS}\s+(?:is|has\s+been|was)\s+"
        r"(?:formally\s+)?(?:proven|proved)\b",
        re.I,
    ),
    re.compile(
        rf"\b{CLAIM_CORRECTNESS}\s+(?:is|has\s+been|was)\s+"
        rf"(?:formally\s+)?{END_TO_END}\s+verified\b",
        re.I,
    ),
    re.compile(
        rf"\b{CLAIM_CORRECTNESS}\s+(?:is|has\s+been|was)\s+"
        rf"(?:formally\s+)?verified\s+{END_TO_END}\b",
        re.I,
    ),
    re.compile(rf"\b{END_TO_END}\s+verified\s+{CLAIM_CORRECTNESS}\b", re.I),
    re.compile(
        r"\b(?:peony\s+)?(?:has\s+)?(?:proved|proven|proves?)\s+"
        r"(?:the\s+)?(?:rust\s+)?(?:linker|implementation)\s+correct\b",
        re.I,
    ),
    re.compile(
        rf"\bpeony\s+{PROOF_ACTION}\s+(?:{END_TO_END}\s+)?{CLAIM_CORRECTNESS}\b",
        re.I,
    ),
    re.compile(
        rf"\bpeony\s+{PROOF_ACTION}\s+{CLAIM_CORRECTNESS}\s+{END_TO_END}\b",
        re.I,
    ),
    re.compile(
        rf"\bpeony\s+{VERIFY_ACTION}\s+(?:{END_TO_END}\s+)?{CLAIM_CORRECTNESS}\b",
        re.I,
    ),
    re.compile(
        rf"\bpeony\s+{VERIFY_ACTION}\s+{CLAIM_CORRECTNESS}\s+{END_TO_END}\b",
        re.I,
    ),
    re.compile(
        rf"\bpeony\s+(?:formally\s+)?{END_TO_END}\s+"
        rf"(?:verifies|verified)\s+{CLAIM_CORRECTNESS}\b",
        re.I,
    ),
    re.compile(r"\b(?:formally\s+)?verified\s+(?:rust\s+)?implementation\b", re.I),
    re.compile(r"\b(?:the\s+)?(?:rust\s+)?implementation\s+is\s+(?:formally\s+)?verified\b", re.I),
    re.compile(r"\bimplementation[-\s]+verified\s+(?:rust\s+)?linker\b", re.I),
    re.compile(r"\bpeony\s+is\s+(?:a\s+)?(?:formally\s+)?verified\b", re.I),
    re.compile(r"\bpeony\s+has\s+(?:a\s+)?(?:formally\s+)?proven[-\s]+correct\s+implementation\b", re.I),
    re.compile(
        r"\bpeony\s+(?:is|has\s+been|was)\s+(?:formally\s+)?"
        r"end[-\s]+to[-\s]+end\s+verified\b",
        re.I,
    ),
    re.compile(r"\bpeony\s+is\s+(?:formally\s+)?verified\s+end[-\s]+to[-\s]+end\b", re.I),
    re.compile(
        r"\b(?:the\s+)?(?:rust\s+)?(?:linker|implementation)\s+"
        r"(?:is|has\s+been|was)\s+(?:formally\s+)?end[-\s]+to[-\s]+end\s+verified\b",
        re.I,
    ),
    re.compile(r"\bend[-\s]+to[-\s]+end\s+verified\s+(?:rust\s+)?(?:linker|implementation)\b", re.I),
    re.compile(
        r"\bpeony\s+(?:(?:has\s+)?(?:formally\s+)?verified|(?:formally\s+)?verifies)\s+"
        r"(?:the\s+)?(?:rust\s+)?(?:linker|implementation)(?:\s+correctness)?\s+"
        r"end[-\s]+to[-\s]+end\b",
        re.I,
    ),
    re.compile(
        r"\bpeony\s+(?:formally\s+)?end[-\s]+to[-\s]+end\s+verifies\s+"
        r"(?:the\s+)?(?:rust\s+)?(?:linker|implementation)(?:\s+correctness)?\b",
        re.I,
    ),
    re.compile(
        r"\bwhole[-\s]+(?:program|linker|implementation)\s+"
        r"(?:is\s+)?(?:formally\s+)?verified\b",
        re.I,
    ),
]
SKIP_PARTS = {".git", ".omo", "target", "verification-assumptions"}


def strip_fenced_code(text: str) -> str:
    kept: list[str] = []
    in_fence = False
    for line in text.splitlines():
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
            kept.append("")
        else:
            kept.append("" if in_fence else line)
    return "\n".join(kept)


def has_skipped_part(root: Path, candidate: Path) -> bool:
    try:
        rel = candidate.relative_to(root)
    except ValueError:
        rel = candidate
    return any(part in SKIP_PARTS for part in rel.parts)


def public_doc_files(path: Path) -> list[Path]:
    if path.is_file():
        return [path]
    files: list[Path] = []
    for relative in ("README.md", "rocq-tests/README.md"):
        candidate = path / relative
        if candidate.is_file():
            files.append(candidate)
    docs = path / "docs"
    if docs.is_dir():
        files.extend(candidate for candidate in sorted(docs.rglob("*.md")) if not has_skipped_part(path, candidate))
    if files:
        return files
    return [candidate for candidate in sorted(path.rglob("*.md")) if not has_skipped_part(path, candidate)]


def main() -> int:
    if len(sys.argv) != 2:
        print("error: expected public doc root or file", file=sys.stderr)
        return 2
    root = Path(sys.argv[1]).resolve()
    if not root.exists():
        print(f"error: wording scan path does not exist: {root}", file=sys.stderr)
        return 2
    violations: list[tuple[Path, int, str]] = []
    for doc in public_doc_files(root):
        text = strip_fenced_code(doc.read_text(encoding="utf-8"))
        for line_no, line in enumerate(text.splitlines(), start=1):
            for pattern in FORBIDDEN:
                if match := pattern.search(line):
                    violations.append((doc, line_no, match.group(0)))
    if violations:
        print("forbidden public verification overclaims found:", file=sys.stderr)
        for doc, line_no, phrase in violations:
            print(f"{doc}:{line_no}: {phrase}", file=sys.stderr)
        return 1
    print("Public wording gate passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
