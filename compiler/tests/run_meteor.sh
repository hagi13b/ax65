#!/bin/sh
# meteor（sample/meteor）を -O0 と -O2 でコンパイルし、シミュレータで動かして、
# 手書きの想定出力 asm/meteor_O0.s と端末への出力がバイト単位で同じかを調べる。
# 1.1 の sample/tools（実機 v2 用のシミュレータ）が必要。
# 使い方：sh tests/run_meteor.sh   （compiler のフォルダで実行）
set -e
cd "$(dirname "$0")/.."
PROJ=../sample/meteor
TOOLS=../sample/tools
REF=../sample/asm/meteor_O0.s

cargo run --release -- build --project "$PROJ" -t v2 -O0 -o build/O0 --emit bin,woz,sym,lst,report
cargo run --release -- build --project "$PROJ" -t v2 -O2 -o build/O2 --emit bin,woz,sym,lst,report

python3 tests/meteor_compare.py --tools "$TOOLS" --ref "$REF" build/O0 build/O2

# meteor7（7セグメント版）も -O2 でコンパイルして比べる
cargo run --release -- build --project ../sample/meteor7 -t v2 -O2 -o build/meteor7_O2 --emit bin,woz,sym,lst,report
python3 tests/meteor7_compare.py "$TOOLS" ../sample/asm/meteor7_O2.s build/meteor7_O2/meteor7
