#!/usr/bin/env python3
"""
Repository inventory + audit candidate normalizer.

Usage examples:
  python3 datasets/list_repos.py --root ~/projects

Outputs:
  - data/project_inventory.json — stats for every git repo under --root (local; gitignored).
  - data/audit_candidates.local.json — validated + rewritten audit candidates (local; gitignored).

This repository also ships a portable example dataset:
  - data/audit_candidates.json

Validation rules for the audit candidates file:
  * query entries must include type, difficulty, expected_snippet, relevant_files.
  * every path in relevant_files must exist relative to repo root.
  * negative_examples[] entries must include query + reason.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List

IGNORE_DIRS = {
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    "coverage",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
    "logs",
    ".next",
    "tmp",
    "temp",
    ".turbo",
    "artifacts",
    "android",
    "ios",
    "Pods",
    ".svn",
    ".hg",
}

LANG_MAP = {
    ".rs": "Rust",
    ".py": "Python",
    ".ts": "TypeScript",
    ".tsx": "TypeScript",
    ".js": "JavaScript",
    ".jsx": "JavaScript",
    ".java": "Java",
    ".kt": "Kotlin",
    ".go": "Go",
    ".c": "C",
    ".h": "C/C++",
    ".hpp": "C/C++",
    ".cc": "C++",
    ".cpp": "C++",
    ".cs": "C#",
    ".swift": "Swift",
    ".m": "Objective-C",
    ".mm": "Objective-C++",
    ".rb": "Ruby",
    ".php": "PHP",
    ".hs": "Haskell",
    ".scala": "Scala",
    ".sh": "Shell",
    ".bash": "Shell",
    ".zsh": "Shell",
    ".fish": "Shell",
    ".sql": "SQL",
    ".html": "HTML",
    ".css": "CSS",
    ".scss": "CSS",
    ".md": "Markdown",
    ".json": "JSON",
    ".yaml": "YAML",
    ".yml": "YAML",
    ".toml": "TOML",
    ".ini": "INI",
    ".cfg": "Config",
    ".dart": "Dart",
    ".lua": "Lua",
    ".vue": "Vue",
    ".svelte": "Svelte",
    ".gradle": "Gradle",
    ".xml": "XML",
    ".bat": "Batch",
    ".ps1": "PowerShell",
    ".pl": "Perl",
    ".erl": "Erlang",
    ".ex": "Elixir",
    ".exs": "Elixir",
    ".mjs": "JavaScript",
    ".cjs": "JavaScript",
    ".proto": "Proto",
    ".tf": "Terraform",
    ".mdx": "MDX",
    ".coffee": "CoffeeScript",
    ".fs": "F#",
    ".fsi": "F#",
    ".ml": "OCaml",
    ".mli": "OCaml",
    ".zig": "Zig",
    ".wasm": "Wasm",
}


@dataclass
class RepoStats:
    name: str
    path: Path
    files: int
    language_count: int
    languages: List[Dict[str, object]]


def find_git_repos(root: Path) -> List[Path]:
    repos: List[Path] = []
    for dirpath, dirnames, _ in os.walk(root):
        if ".git" in dirnames:
            repos.append(Path(dirpath))
            dirnames.remove(".git")
        dirnames[:] = [d for d in dirnames if d not in IGNORE_DIRS]
    return sorted(set(repos))


def collect_repo_stats(repo: Path) -> RepoStats:
    file_count = 0
    lang_counts: Dict[str, int] = defaultdict(int)
    for dirpath, dirnames, filenames in os.walk(repo):
        dirnames[:] = [d for d in dirnames if d not in IGNORE_DIRS]
        for filename in filenames:
            file_count += 1
            ext = os.path.splitext(filename)[1].lower()
            lang = LANG_MAP.get(ext)
            if lang:
                lang_counts[lang] += 1
    top_langs = sorted(lang_counts.items(), key=lambda kv: kv[1], reverse=True)
    lang_total = sum(lang_counts.values()) or 1
    languages = [
        {
            "language": lang,
            "files": count,
            "share": round(count / lang_total, 4),
        }
        for lang, count in top_langs[:6]
    ]
    return RepoStats(
        name=repo.name,
        path=repo,
        files=file_count,
        language_count=len(top_langs),
        languages=languages,
    )


def write_inventory(stats: Iterable[RepoStats], output: Path) -> None:
    payload = [
        {
            "name": stat.name,
            "path": str(stat.path),
            "files": stat.files,
            "language_count": stat.language_count,
            "languages": stat.languages,
        }
        for stat in stats
    ]
    output.write_text(json.dumps(payload, ensure_ascii=False, indent=2))


def validate_candidates(candidates_path: Path) -> None:
    if not candidates_path.exists():
        print(
            f"[WARN] candidates file {candidates_path} not found, skipping validation",
            file=sys.stderr,
        )
        return
    candidates = json.loads(candidates_path.read_text())
    for entry in candidates:
        repo_path = Path(entry["path"]).expanduser()
        if not repo_path.exists():
            raise FileNotFoundError(f"repo path missing: {repo_path}")
        for query in entry.get("queries", []):
            for field in ("query", "type", "difficulty", "expected_snippet", "relevant_files"):
                if field not in query:
                    raise ValueError(f"query {entry['name']} missing field {field}")
            for rel in query["relevant_files"]:
                file_path = repo_path / rel
                if not file_path.exists():
                    raise FileNotFoundError(
                        f"missing file {file_path} for query {query['query']}"
                    )
        for neg in entry.get("negative_examples", []):
            if "query" not in neg or "reason" not in neg:
                raise ValueError(
                    f"negative example malformed for {entry['name']}: {neg}"
                )
    candidates.sort(key=lambda entry: entry["name"])
    candidates_path.write_text(json.dumps(candidates, ensure_ascii=False, indent=2))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Scan repositories and normalize audit candidates"
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path.cwd(),
        help="Root folder that contains git repositories (defaults to current working directory)",
    )
    parser.add_argument(
        "--inventory",
        type=Path,
        default=Path("data/project_inventory.json"),
        help="Output path for repository stats",
    )
    parser.add_argument(
        "--candidates",
        type=Path,
        default=Path("data/audit_candidates.local.json"),
        help="Audit candidates file to validate and rewrite",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    repos = find_git_repos(args.root)
    if not repos:
        raise SystemExit(f"no git repos found under {args.root}")
    stats = [collect_repo_stats(repo) for repo in repos]
    args.inventory.parent.mkdir(parents=True, exist_ok=True)
    write_inventory(stats, args.inventory)
    validate_candidates(args.candidates)
    print(f"Inventory written for {len(stats)} repos -> {args.inventory}")
    print(f"Candidates validated -> {args.candidates}")


if __name__ == "__main__":
    main()
