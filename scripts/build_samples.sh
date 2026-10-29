#!/usr/bin/env sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
COMPILER_DIR="$ROOT_DIR/compiler"
AX65_BIN="${AX65_BIN:-$COMPILER_DIR/target/release/ax65}"
STD_DIR="$COMPILER_DIR/std"
LOG_DIR="$ROOT_DIR/sample/build_logs"

export AX65_STD="$STD_DIR"

mkdir -p "$LOG_DIR"

if [ ! -x "$AX65_BIN" ] && [ ! -x "${AX65_BIN}.exe" ]; then
  printf '%s\n' '[build_samples] compiler binary not found; building it first...'
  (cd "$COMPILER_DIR" && cargo build --release) >/dev/null 2>&1 || {
    printf '%s\n' '[build_samples] cargo build failed' >&2
    exit 1
  }
fi

if [ ! -x "$AX65_BIN" ] && [ ! -x "${AX65_BIN}.exe" ]; then
  printf '%s\n' '[build_samples] ax65 binary is still missing after build.' >&2
  exit 1
fi

# Sample projects that are expected to compile as standalone AX65 projects.
SAMPLE_PROJECTS="
$ROOT_DIR/sample/disasm
$ROOT_DIR/sample/meteor
$ROOT_DIR/sample/meteor7
"

SUMMARY=''
TOTAL=0
OK=0
FAIL=0

for project_dir in $SAMPLE_PROJECTS; do
  project_name=$(basename "$project_dir")
  project_log_dir="$LOG_DIR/$project_name"
  mkdir -p "$project_log_dir"

  TOTAL=$((TOTAL + 1))

  printf '%s\n' "[build_samples] building $project_name"

  case "$project_name" in
    meteor)
      levels='O0 O1 O2 Os'
      ;;
    *)
      levels='O2'
      ;;
  esac

  project_ok=1
  for level in $levels; do
    out_dir="$project_dir/build/$level"
    mkdir -p "$out_dir"
    "$AX65_BIN" build --project "$project_dir" -t v2 -"$level" -o "$out_dir" --emit bin,woz,sym,lst,report > "$project_log_dir/${level}.log" 2>&1 || {
      project_ok=0
      printf '%s\n' "[build_samples] failed: $project_name -$level" >&2
    }
  done

  if [ "$project_ok" -ne 1 ]; then
    FAIL=$((FAIL + 1))
    SUMMARY="$SUMMARY\n  FAIL $project_name"
    continue
  fi

  "$AX65_BIN" check --project "$project_dir" --all-targets > "$project_log_dir/check.log" 2>&1 || {
    project_ok=0
    printf '%s\n' "[build_samples] check failed: $project_name" >&2
  }

  if [ "$project_ok" -eq 1 ]; then
    OK=$((OK + 1))
    SUMMARY="$SUMMARY\n  OK   $project_name"
  else
    FAIL=$((FAIL + 1))
    SUMMARY="$SUMMARY\n  FAIL $project_name"
  fi
done

printf '%s\n' ''
printf '%s\n' '[build_samples] summary'
printf '%b\n' "$SUMMARY"
printf '%s\n' "total=$TOTAL ok=$OK fail=$FAIL"

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

exit 0
