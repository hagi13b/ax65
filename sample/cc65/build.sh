#!/bin/sh
# build.sh - meteor の cc65 版を 5 つの最適化の設定で作る（v2 用。仕様書 1.1）
#   必要なもの：cc65 一式（cl65、cc65、ca65、ld65）。2.19 以降を想定
#   出力：meteor_<設定>.bin / .map / .lbl / .s（.s はコンパイラが出したアセンブリ）
set -e
CPU="-t none --cpu 65c02"
build() {   # $1 = 設定の名前、$2 以降 = cc65 の最適化のオプション
    name=$1; shift
    cc65 $CPU "$@" -o meteor_$name.s meteor.c                       # C → アセンブリ（確認用）
    cl65 $CPU "$@" -C meteor.cfg -o meteor_$name.bin \
         -m meteor_$name.map -Ln meteor_$name.lbl crt0.s meteor.c   # 組み立てとリンク
    echo "built meteor_$name.bin"
}
build O0                    # 最適化なし
build O      -O             # 標準の最適化
build Oirs   -Oirs          # 最適化＋展開（inline）＋register 変数＋標準関数の展開
build Oirs_Cl -Oirs -Cl     # さらにローカル変数を static に置く（cc65 で最も速くなりやすい組み合わせ）
build Oirs_Cl_cs -Oirs -Cl --codesize 500   # 展開の上限をさらに上げる（速さ優先）
cc65 --version
