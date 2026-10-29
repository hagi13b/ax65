# AX65 1.1 引き継ぎメモ（2026-10-29、3 回目のチャットの終わり）

次のチャットでは、最初にこのファイルを読ませてください。作業フォルダは `C:\Users\a1617002\Documents\ax65\`（リポジトリのルート）で、`archive/` は履歴資料です。

## 現在の状態

| フォルダ | 内容 | 状態 |
|---|---|---|
| `spec/ax65_spec_1.1.md` | 言語仕様書 1.1 | 完成。元の原稿は `../archive/spec_parts/` の p0, p1〜p9, p10 の順に cat で連結して作る（p10 は最後） |
| `sample/` | METEOR DODGE、METEOR7、想定出力（`asm/*.s`）、ツール、測定結果 | 完成。cc65 版の測定も済み（報告書 11 章、仕様書 付録D） |
| `manual/ax65_manual_1.1.md` | プログラミング入門 | 完成 |
| `compiler/` | Rust のリファレンスコンパイラ | **ビルドが通り、警告 0 件、`cargo test` 23 件成功。meteor（-O0〜-Os）と METEOR7 の出力は想定出力と一致** |
| `editors/` | 色分けの設定 | 完成 |

## コンパイラの大きさ（meteor、v2、コードのバイト数）

| 段階 | 1.1 の最初 | 今 | 想定出力 |
|---|---|---|---|
| -O0 | 619 | **487** | 493 |
| -O1 | 554 | 426 | 368 |
| -O2 | 503 | 379 | 312 |
| -Os | 554（-O1 と同じ） | 426（-O1 と同じ） | 352 |
| METEOR7 -O2 | 481 | 396 | 268 |

## この回にしたこと

1. cc65 版の測定（最速 `-Oirs -Cl --codesize 500`：1 フレーム 2148.5 サイクルで AX65 -O2 の 2.82 倍、コード 821 バイトで 2.63 倍）。報告書 11 章と仕様書 付録D に反映。
2. コンパイラの警告 32 件を整理（使わない関数・フィールドを削除。一部は使う形に：未使用の引数の警告文、E0201 のモジュール名、.lst に変数の領域）。
3. のぞき穴最適化 `src/peep.rs`（-O1 以上）：分岐の整理、レジスタの中身の追跡（合流点での引き継ぎ、キャリー・定数の分かっている分岐）、ローカル変数への読まれない書き込みの削除。アセンブリと cycles() の中は `Item::Mark("KEEP{")`〜`("}KEEP")` で囲んで変えない。
4. コード生成（-O0 の土台）の見直し（codegen.rs・cgexpr.rs）：
   - 命令を出す段階で A・X・Y の中身を覚える（読み直しを出さない、`tax`・`txa` にする、N・Z だけが違う lda は次の命令を見て決める、前からの分岐だけが来るラベルで中身を引き継ぐ）
   - 呼ばれる関数から先に作り、関数ごとに壊すレジスタを記録（`Gen.clob`）。呼んでも壊れないレジスタに、ループの番号・添字の引数・文字列ループの数を置く
   - `?`・`if let` の値を A のまま使う、match の最後の腕は比べない、`if c { break; }` は直接飛ぶ、中身のない while は分岐 1 つ、`x > 0` は `!= 0`
   - 読まれないローカル変数への書き込みは -O0 でも消す（`peep::dead_frame_stores`）

## 次にやること

1. **-O1・-O2・-Os を想定出力に近づける**（今は -O0 の改善がそのまま効いている程度）。想定出力が前提にしている未実装のもの：
   - -O2 の「関数ごとの引数の渡し方の変更」（respawn の m を X で渡し、update_meteors のループの番号を X に置く）
   - 1 か所からしか呼ばれない関数を展開した後の、引数・戻り値のレジスタでの受け渡し（read_key → match → move_player が直接つながる形）
   - `digits()` の共通部分のくくり出し（print_score の print_bcd）、-O2 の「ループをまたぐ不要な書き込みの削除」（clear_meteors の wait）
   - -Os の展開の拡大（今は -O1 と同じコードになる）
2. 未実装：NMOS のコード生成、`strbuf`、`text`、`io_span`。
3. 実機で確かめること：T2CH を書いたときに T2 のフラグが消えるか、クロックが 1.8425MHz か 1.8432MHz か、HEXMON の API のスタックの量（今は 4 バイトと仮定）。
4. v3 の VRAM の仕様が決まったら `targets/v3.target` と `hw/v3_screen.ax65` を直す。

## ビルドと確認の手順

この作業の環境には Rust がなく、インターネットにも出られないので、ビルドはユーザーの Windows で行う。

1. `compiler\tests\win_build.bat` をダブルクリック（ビルド、`cargo test`、meteor の 4 段階、全ターゲットの検査、METEOR7）。結果は `compiler\logs\` と `compiler\build\` に出る。
2. 出力の比較（Python だけでできる）：
   ```
   cd compiler
   python3 -I tests/meteor_compare.py --tools ../sample/tools --ref ../sample/asm/meteor_O0.s build/O0 build/O1 build/O2 build/Os
   python3 -I tests/meteor7_compare.py ../sample/tools ../sample/asm/meteor7_O2.s build/meteor7_O2/meteor7
   ```
3. 想定出力の .bin などは `cd sample; python3 -I tools/run_bench.py` で、cc65 版は `cc65/build.sh` でビルドしてから `python3 -I tools/run_cc65.py cc65` で作り直せる（zip には、バイナリと一次出力を入れていない）。

## 途中の版の保存場所（archive/）

- `compiler_src_1.1_before_cleanup/`：警告を整理する前のソース
- `src_before_codegen_r1/`、`src_codegen_r1_ok/`：コード生成の見直しの前と 1 回目の codegen.rs・cgexpr.rs
- `peep_v1_ok.rs`：のぞき穴最適化の最初の版

## 決まりごと（ユーザーの希望）

- 仕様書の本文には結論だけを書く。理由は Q&A に書いてリンクする。
- アセンブリの例は、すべての行に日本語のコメントを付ける。
- 実機は v2：6502-CPLD ボード＋HEXPAD、モニタは HEXMON。資料は `../archive/ref/hexmon_guide_2.6.md`。割り込みはない。T1 と ACR はモニタが使う。
- 確認のための質問は作業前に行う。許可の確認が出る操作は少なくする。
