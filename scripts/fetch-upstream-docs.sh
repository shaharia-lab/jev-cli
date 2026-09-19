#!/usr/bin/env bash
# Mirror the TypeSafe docs as Markdown into docs/upstream/ for local use as development context.
# The mirror is gitignored: the content belongs to TypeSafe and is not redistributed by this repo.
set -euo pipefail

BASE="https://docs.typesafe.ai"
OUT="$(cd "$(dirname "$0")/.." && pwd)/docs/upstream"

mkdir -p "$OUT"
curl -fsSL "$BASE/llms.txt" -o "$OUT/llms.txt"

{
  grep -oE "$BASE/[^)]+\.md" "$OUT/llms.txt"
  echo "$BASE/migrating-to-v1.md" # linked from the agent skill, missing from the index
} | sort -u | while read -r url; do
  dest="$OUT/${url#"$BASE"/}"
  mkdir -p "$(dirname "$dest")"
  curl -fsSL --max-time 30 "$url" -o "$dest" || echo "FAILED $url" >&2
done

# Pages embed a ~230-line JSX helper for playground links; drop it so the files read as prose.
python3 - "$OUT" <<'PY'
import pathlib, re, sys

for path in pathlib.Path(sys.argv[1]).rglob("*.md"):
    text = path.read_text()
    start = text.find("export function TypesafeExample")
    if start == -1:
        continue
    end = re.search(r"\n}\n", text[start:])
    if end:
        path.write_text(text[:start] + text[start + end.end():])
PY

echo "Mirrored $(find "$OUT" -name '*.md' | wc -l) pages into $OUT"
