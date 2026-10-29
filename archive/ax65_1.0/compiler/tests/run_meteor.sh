#!/bin/sh
# meteor（sample/project）を -O0 と -O2 でコンパイルし、エミュレータで動かして、
# 手書きの想定出力 asm/meteor_O0.s と端末への出力がバイト単位で同じかを調べる。
# 使い方：sh tests/run_meteor.sh   （ax65_compiler のフォルダで実行）
set -e
cd "$(dirname "$0")/.."
PROJ=../sample/project
TOOLS=../sample/tools
REF=../sample/asm/meteor_O0.s

cargo run --release -- build --project "$PROJ" -t v2 -O0 -o build/O0 --emit bin,woz,sym,lst,report
cargo run --release -- build --project "$PROJ" -t v2 -O2 -o build/O2 --emit bin,woz,sym,lst,report

python3 tests/meteor_compare.py --tools "$TOOLS" --ref "$REF" build/O0 build/O2
