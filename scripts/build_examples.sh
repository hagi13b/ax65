#!/usr/bin/env sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
COMPILER_DIR="$ROOT_DIR/compiler"
AX65_BIN="${AX65_BIN:-$COMPILER_DIR/target/release/ax65}"
STD_DIR="$COMPILER_DIR/std"
LOG_DIR="$ROOT_DIR/manual/examples/build_logs"

export AX65_STD="$STD_DIR"

mkdir -p "$LOG_DIR"

if [ ! -x "$AX65_BIN" ] && [ ! -x "${AX65_BIN}.exe" ]; then
  printf '%s\n' '[build_examples] compiler binary not found; building it first...'
  (cd "$COMPILER_DIR" && cargo build --release) >/dev/null 2>&1 || {
    printf '%s\n' '[build_examples] cargo build failed' >&2
    exit 1
  }
fi

if [ ! -x "$AX65_BIN" ] && [ ! -x "${AX65_BIN}.exe" ]; then
  printf '%s\n' '[build_examples] ax65 binary is still missing after build.' >&2
  exit 1
fi

SUMMARY=''
TOTAL=0
OK=0
FAIL=0

PROJ_LIST_FILE=$(mktemp)
find "$ROOT_DIR/manual/examples" -name ax65.proj -print | sort > "$PROJ_LIST_FILE"

while IFS= read -r proj; do
  project_dir=$(dirname "$proj")
  project_name=$(basename "$project_dir")
  TOTAL=$((TOTAL + 1))

  printf '%s\n' "[build_examples] building $project_name"

  if "$AX65_BIN" build --project "$project_dir" -t v2 -o "$project_dir/build" --emit bin,woz,sym,lst,report > "$LOG_DIR/${project_name}.log" 2>&1; then
    if "$AX65_BIN" check --project "$project_dir" --all-targets > "$LOG_DIR/${project_name}_check.log" 2>&1; then
      OK=$((OK + 1))
      SUMMARY="$SUMMARY\n  OK   $project_name"
      continue
    fi
  fi

  FAIL=$((FAIL + 1))
  SUMMARY="$SUMMARY\n  FAIL $project_name"
  printf '%s\n' "[build_examples] failed: $project_name" >&2
done < "$PROJ_LIST_FILE"

rm -f "$PROJ_LIST_FILE"

printf '%s\n' ''
printf '%s\n' '[build_examples] summary'
printf '%b\n' "$SUMMARY"
printf '%s\n' "total=$TOTAL ok=$OK fail=$FAIL"

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

exit 0
