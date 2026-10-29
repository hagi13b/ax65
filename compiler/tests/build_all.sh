#!/usr/bin/env sh
set +e

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ROOT_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

cd "$ROOT_DIR"
mkdir -p logs

printf '%s\n' '[1/4] cargo build --release'
cargo build --release > logs/build.txt 2>&1
build_status=$?
printf '%s\n' "exit=$build_status" >> logs/build.txt

if [ ! -x "$ROOT_DIR/target/release/ax65" ] && [ ! -x "$ROOT_DIR/target/release/ax65.exe" ]; then
  printf '%s\n' 'Build failed; skipping remaining steps.'
  printf '%s\n' 'Done. Results are in compiler/logs and compiler/build.'
  exit "$build_status"
fi

printf '%s\n' '[2/4] cargo test'
cargo test --release > logs/test.txt 2>&1
test_status=$?
printf '%s\n' "exit=$test_status" >> logs/test.txt

printf '%s\n' '[3/4] meteor -O0 -O1 -O2 -Os'
for level in O0 O1 O2 Os; do
  out_path="build/$level"
  mkdir -p "$out_path"
  target/release/ax65 build --project ../sample/meteor -t v2 -"$level" -o "$out_path" --emit bin,woz,sym,lst,report > "logs/meteor_${level}.txt" 2>&1
  status=$?
  printf '%s\n' "exit=$status" >> "logs/meteor_${level}.txt"
done

target/release/ax65 check --project ../sample/meteor --all-targets > logs/meteor_check.txt 2>&1
check_status=$?
printf '%s\n' "exit=$check_status" >> logs/meteor_check.txt

printf '%s\n' '[4/4] meteor7 -O2'
target/release/ax65 build --project ../sample/meteor7 -t v2 -O2 -o build/meteor7_O2 --emit bin,woz,sym,lst,report > logs/meteor7_O2.txt 2>&1
meteor7_status=$?
printf '%s\n' "exit=$meteor7_status" >> logs/meteor7_O2.txt

printf '%s\n' 'Done. Results are in compiler/logs and compiler/build.'
exit 0
