# ax65 — AX65 言語のリファレンスコンパイラ（マイルストーン 1）

AX65（仕様書 `../spec/ax65_spec_1.0.md`）のコンパイラ `ax65` です。W65C02S を対象に、プロジェクトのソースを全部読み込み、
6502 の機械語・リスティング・記号表・レポートを出力します。Rust（edition 2021）で、標準ライブラリだけを使っています。

## ビルド

```
cargo build --release
```

実行ファイルは `target/release/ax65` です。標準ライブラリ（`std/`）は、次の順に探します。

1. 環境変数 `AX65_STD`
2. 実行ファイルの隣（`std/`、`../std/`、`../../std/`）
3. ビルドしたときのソースの `std/`（`cargo run` で使う場合）

## 使い方

```
ax65 build --project ../sample/project -t v2 -O0 -o build/O0
ax65 build -t v2                       # 現在のフォルダの ax65.proj、既定の -O2
ax65 build -t v2 -Os --emit bin,lst
ax65 build -t v2 -D debug -D level=3
ax65 build -t v2 --bind console=sim::console
ax65 check --all-targets --deny-warnings
ax65 check --all-levels
ax65 explain E0412
ax65 targets
ax65 new hello
ax65 fmt --check
ax65 --version / --help
```

出力（`-o` を省くと `<out_dir>/<ターゲット名>/`）：`<プロジェクト名>.bin` `.woz` `.hex` `.rom` `.ptp` `.sym` `.lst` `.report`。
`--emit` がなければターゲット定義の `output`、それもなければ bin・sym・lst。レポートは `--no-report` がなければ常に出します。

## テスト

```
cargo test                    # 字句解析・構文解析・アセンブラ・アセンブリの解析の単体テスト
sh tests/run_meteor.sh        # meteor を -O0 と -O2 でコンパイルし、エミュレータで実行して想定出力と比べる
```

`tests/run_meteor.sh` は、`sample/tools/sim65c02.py` で、コンパイルした `.bin` を読み込み先から実行します。
キー入力は `run_bench.py` と同じ（`KEYS`、`QUIT_FRAME`、当たりで終わる `KEYS_HIT`）です。コンパイルした版には
`bench_` のラベルがないので、フレームの区切りは `.sym` の `TICKS` が変わった時点とします（手書きの想定出力も同じ方法で動かして比べます）。
端末への出力がバイト単位で一致するかと、1 フレームの平均サイクル数（`TICKS` が変わってから次の `wai` まで、割り込みの分を除く）を表示します。

## 実装の範囲（仕様書の章との対応）

凡例：○ 実装済み　△ 一部　× 未実装（構文は解析し、使うと `E0000`「未実装」を出す）

| 章 | 内容 | 状況 | 補足 |
|---|---|---|---|
| 2 | 字句（コメント、識別子、キーワード、整数・小数・文字・文字列のリテラル、エスケープ、ラベル） | ○ | ASCII 以外の文字の検査（E0106）、`mut`（E0103） |
| 3.1–3.2 | 整数型・bool、暗黙の変換なし、符号の混在、u8±i8 | ○ | |
| 3.3 | char と変換表（`'A'@screen`） | ○ | |
| 3.4 | 範囲型、check・wrap・unchecked・range | ○ | 16 ビットの範囲型の check は下限 0 か 1 だけ |
| 3.5–3.6 | 列挙型、ビットフラグ（contains・insert・remove・toggle、tsb/trb） | ○ | |
| 3.7–3.8 | 構造体、配列（フィールドごと・下位上位に分けた並べ方）、2 次元配列（行の表） | △ | 2 次元は 1 バイトの要素だけ。#[packed] の配列の変数の添字は × |
| 3.9 | str・zstr・histr・[char; N] | △ | `for c in s` は str だけ。strbuf の操作は × |
| 3.10 | ポインタ（p[i]、p.field、add、add_pages） | △ | 1 バイトでない要素の変数の添字は × |
| 3.11 | Option、Some/None、?、if let、match | ○ | 変数の Option は値＋有無の 1 バイト（使われない値の流用は ×） |
| 3.12 | fnset（jmp (abs,x)） | △ | A だけで引数を渡す関数だけ |
| 3.13–3.14 | type、as の規則 | ○ | |
| 4 | const・static・let、初期化（stz・即値・コピー）、#[zp]、#[noinit]、重ね合わせ、アドレスの規則 | △ | 値を入れる前の読み込みは変数単位（配列の要素ごとの検査は ×）。呼び出し先を通したアドレスの持ち出し、const を指すポインタの書き込みの検査は × |
| 5 | 演算子の優先順位、あふれ、ビット演算、シフト、比較、BCD、定数の掛け算・2 のべき乗の割り算、ブロック式、組み込みのメソッド | △ | 符号付きの / % と saturating・checked は × |
| 6 | if・loop・while（#[max]）・for（範囲・rev・型・配列・文字列）・break/continue/ラベル・match（比較の連鎖、#[jump_table]）・return | △ | 符号付きの範囲の for、16 ビットの rev は × |
| 7 | 関数、const fn、comptime fn、asm fn、再帰の禁止、extern fn、#[inline] 系 | ○ | |
| 8 | モジュール、use（as・as _・{…}）、pub、interface/impl/bind、init の順序、使われないコードの除去 | ○ | |
| 9 | 属性 | ○ | |
| 10 | 割り込み処理（使うレジスタだけ退避）、BRK の振り分け、文脈の規則、irq_enable の規則、shared の規則、atomic | ○ | |
| 11 | I/O（in・out・rmw、読み捨ての bit、所有者、アドレスの重なり） | ○ | |
| 12 | cycles・max_cycles・delay・wait_interrupt・core::us | △ | cycles と max_cycles は分岐のない本体だけ。io_span は × |
| 13 | const の計算、comptime fn（int・real・数学関数）、表の生成、include_bin、comptime assert、target.*・build.*、comptime if/match | ○ | |
| 14 | asm ブロック・asm (in/out)・asm fn・unsafe asm・smc の検査 | ○ | ループの回数の上限の解析は × |
| 15 | 起動と終了、引数（A・X・Y と引数領域）、戻り値（キャリー・A・AX・AXY・結果領域）、スタックの予算 | ○ | |
| 16 | ターゲット定義（extends、region、reserve、place const/code、encoding、bind、const、output ほか） | ○ | place は code だけを使う |
| 17 | プロジェクトファイル、モジュールの探し方、コマンド、オプション、出力形式、リスティング | △ | fmt は 16進の書き方だけ。--json は最小限 |
| 18 | エラー（E0101〜E1210）、警告、メッセージの書式、レポート | △ | reversible_loop の助言、最も長い atomic のサイクル数は × |
| 19 | 最適化の段階 | △ | 下の表 |
| 20 | 組み込み関数、core（us・mul8・mul16・divmod8・divmod16・Nz8・Nz16・QR8・QR16）、bcd、標準のインターフェース | △ | text（数値の文字列化）は × |
| 15.6 | NMOS 6502 | × | NMOS のターゲットではコード生成を行わない（E0000）。asm の 65C02 専用の命令は E1204 |

### 最適化（19章）

| 内容 | -O0 | -O1 | -O2 | -Os |
|---|---|---|---|---|
| 末尾呼び出しの jmp、読み捨ての bit、初期値 0 の stz | ○ | ○ | ○ | ○ |
| #[inline] と init の展開 | ○ | ○ | ○ | ○ |
| 1 か所からしか呼ばれない関数の展開 | | ○ | ○ | ○ |
| 小さな関数（12 命令以下）の展開 | | ○ | | ○ |
| ゼロページへの自動の配置（static とローカル変数の領域） | | ○ | ○ | ○ |
| 引数の渡し方の変更、不要な書き込みの削除、共通部分のくくり出し、ループの自動の逆順化・配置、構造体の戻り値のレジスタ返し | × | × | × | × |

## 既知の制限

- NMOS 6502 のターゲットのコード生成は未実装です。
- `strbuf`、`text`、`io_span()` は未実装です（型の検査は行い、コード生成で E0000）。
- 割り込み処理の最長サイクル数は、ループがない場合だけ上限を計算します。
- メモリの配置は、プログラム（コード＋定数）と変数を、読み込み先を含む 1 つの ram の区画に続けて置きます（複数の区画にまたがる配置はしません）。
- `fmt` は 16進の書き方（`$FF`／`0xFF`）だけをそろえます。
- 値を入れる前の読み込みの検査は、ループの中で入れた値をループの後でも入っているとみなす寛容な扱いです。
- ループ変数への代入はエラー（E0512）にしています。

## ファイル構成

```
Cargo.toml
src/main.rs        入口
src/cli.rs         コマンドとオプション（17.4節・17.5節）
src/diag.rs        エラー・警告とその書式（18章）、ソースの管理
src/explain.rs     ax65 explain の説明
src/lexer.rs       字句解析（2章）
src/ast.rs         構文木
src/parser.rs      構文解析（付録A）
src/project.rs     ax65.proj とターゲット定義（16章・17.2節）
src/program.rs     モジュールの読み込み（17.3節）、名前の表、中心の文脈 Ctx、条件コンパイル
src/types.rs       型
src/sema.rs        宣言の登録、型と名前の解決、呼び出しグラフ・shared・I/O などプログラム全体の検査
src/check.rs       関数の本体の型検査
src/comptime.rs    コンパイル時の計算（13章）
src/codegen.rs     コード生成：関数・文・制御構文・起動処理
src/cgexpr.rs      コード生成：値・式・呼び出し・Option・for・インラインアセンブリ
src/opt.rs         展開、末尾呼び出し、レジスタの退避、重ね合わせ、ゼロページの配置、アセンブル、スタックの予算
src/asm6502.rs     2 パスのアセンブラ（命令表とサイクル数、届かない分岐の書き換え、リスティング）
src/asmtext.rs     インラインアセンブリのテキストの解析
src/output.rs      bin・woz・hex・rom・ptp・sym・lst
src/report.rs      レポート（18.4節）
std/               標準ライブラリ（core、bcd、console、screen、keyboard、timer、mul、mul_shift）
tests/             run_meteor.sh と meteor_compare.py
```
