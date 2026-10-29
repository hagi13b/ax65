#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${OUT_DIR:-$ROOT_DIR/dist}"
NAME="${AX65_PACKAGE_NAME:-ax65_clean_release}"
WITH_ARCHIVE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -o|--out)
      OUT_DIR="$2"
      shift 2
      ;;
    -n|--name)
      NAME="$2"
      shift 2
      ;;
    --with-archive)
      WITH_ARCHIVE=1
      shift
      ;;
    -h|--help)
      cat <<EOF
Usage: $(basename "$0") [options]

Options:
  -o, --out PATH        Output directory for the zip file (default: dist/)
  -n, --name NAME       Base filename for the archive (default: ax65_clean_release)
      --with-archive    Include archive/ in the package
  -h, --help            Show this help
EOF
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 2
      ;;
  esac
done

mkdir -p "$OUT_DIR"
STAMP="$(date +%Y%m%d-%H%M%S)"
ARCHIVE_PATH="${OUT_DIR}/${NAME}-${STAMP}.zip"

python3 - "$ROOT_DIR" "$ARCHIVE_PATH" "$WITH_ARCHIVE" <<'PY'
import sys
import zipfile
from pathlib import Path

root = Path(sys.argv[1]).resolve()
out_zip = Path(sys.argv[2]).resolve()
with_archive = sys.argv[3] == "1"

exclude_dirs = {
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    "target",
    "dist",
    "build",
    "out",
    "tmp",
    ".idea",
}
exclude_names = {".DS_Store", "Thumbs.db"}
exclude_ext = {
    ".bin", ".rom", ".woz", ".ptp", ".hex", ".sym", ".lst",
    ".report", ".o", ".obj", ".a", ".so", ".dylib", ".dll",
    ".exe", ".pyc", ".pyo", ".log", ".tmp", ".bak", ".orig",
    ".zip", ".gz", ".tgz", ".bz2", ".xz", ".7z", ".tar",
    ".rar", ".pdf", ".png", ".jpg", ".jpeg", ".gif", ".webp",
    ".ico", ".mp3", ".wav", ".mp4", ".mov"
}

with zipfile.ZipFile(out_zip, "w", compression=zipfile.ZIP_DEFLATED) as zf:
    for file in sorted(root.rglob("*")):
        if not file.is_file():
            continue

        rel = file.relative_to(root)
        parts = rel.parts

        if any(part in exclude_dirs for part in parts):
            continue
        if any(part.startswith(".git") for part in parts):
            continue
        if rel.parts and rel.parts[0] == ".vscode":
            continue
        if rel.parts and rel.parts[0] == "archive" and not with_archive:
            continue
        if file.name in exclude_names:
            continue
        if rel.suffix.lower() in exclude_ext:
            continue
        zf.write(file, arcname=rel.as_posix())
PY

echo "Created clean archive: $ARCHIVE_PATH"
