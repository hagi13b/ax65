# ax65 — AX65 言語のリファレンスコンパイラ 1.1

AX65（仕様書 `../spec/ax65_spec_1.1.md`）のコンパイラ `ax65` 1.1.0 です。W65C02S を対象に、プロジェクトのソースを全部読み込み、
6502 の機械語・リスティング・記号表・レポートを出力します。Rust（edition 2021）で、標準ライブラリだけを使っています。

## ビルド

Rust（stable、edition 2021）が必要です。外部のクレートは使いません。

```
cargo build --release
```

実行ファイルは `target/release/ax65`（Windows では `target\release\ax65.exe`）です。`ax65 --version` で版を確かめられます。標準ライブラリ（`std/`）は、次の順に探します。

1. 環境変数 `AX65_STD`
2. 実行ファイルの隣（`std/`、`../std/`、`../../std/`）
3. ビルドしたときのソースの `std/`（`cargo run` で使う場合）

## 使い方

```
ax65 build --project ../sample/meteor -t v2 -O0 -o build/O0
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
sh tests/run_meteor.sh        # meteor を -O0 と -O2 でコンパイルし、シミュレータで実行して想定出力と比べる
```

`tests/run_meteor.sh` は、`../sample/meteor`（プロジェクト）を `-t v2` でコンパイルし、`../sample/tools` のシミュレータで
コンパイルした `.bin` と、手書きの想定出力 `../sample/asm/meteor_O0.s`（`asm65.py` で組み立てる）を動かして、端末（UART）への出力を比べます。

- **1.1 の sample/tools（実機 v2 用に書き直したシミュレータ）が必要です。** 1.0 の tools（VIA $6000・ACIA $8000・Wozmon $FF1F の旧 v2）では動きません。
- `meteor_compare.py` が tools に求めるのは、`asm65.Asm().run()`（`.out`・`.sym`）と、`sim65c02.Machine`（`.mem`・`.PC`・`.S`・`.cyc`・`.step()`・受信の `.rx` か `.feed()`・送信の `.out`）だけです。
  `sim65c02.SENTINEL` があればそれを、なければ $FFF0 を戻り番地としてスタックに積んでから `start` を実行し、`exit = rts` で戻ってきたら終わりとします（HEXMON の JMP キーと同じ形）。
- v2 は `irq = none` なので、フレームの区切り（1.0 の `TICKS`）は使いません。キーは「それまでの出力のバイト数」を合図に送り（`--keys 200:a,400:d`、`--quit 3000`）、出力の一致と総サイクル数を表示します。

## 1.1 での変更（仕様書 付録C・付録B の N）

| 内容 | 状況 | 実装の場所 |
|---|---|---|
| `exit = rts`：起動処理はスタックポインタを設定せず、`jmp main`（main の `rts` でモニタへ戻る。15.1節・16.4節） | ○ | project.rs（解析）、codegen.rs `gen_start` |
| `irq = none`・`nmi = none`、省略したときも none（16.4節） | ○ | project.rs `VecMode::None`・`Target::has_irq`／`has_nmi` |
| none のターゲットでの割り込みの機能のエラー E1110：`#[interrupt(irq/brk/nmi)]`、`irq_enable()`、`wait_interrupt()`、`shared(irq)`・`shared(nmi)`、両方 none の `shared`、`irq_interval` | ○ | sema.rs `ensure_sig`・`ensure_global`、check.rs `check_builtin_call`、project.rs `validate_target` |
| `wait_interrupt()` は W65C02S かつ irq が none でないときだけ（12.3節） | ○ | check.rs（E1105・E1110） |
| irq・nmi がどちらも none なら `atomic`・`cycles()` の暗黙の atomic は命令を生成しない（10.5節） | ○ | codegen.rs `gen_atomic`・`gen_timed` |
| `target.has_irq`・`target.has_nmi`（13.5節。`comptime if` の両方で使える） | ○ | program.rs `target_field` |
| `reserve` はどの区画の中にも書け、1 つのアドレス（`reserve $800B;`）も書ける。`extends` では親の範囲に足される（16.1節・16.3節） | ○ | project.rs `reserves_of`・`merged_onto` |
| indirect・jump のベクタの番地は自動で reserve に加わる（16.4節） | ○ | project.rs `TargetSet::resolve` |
| reserve の中の I/O 変数は E1111（16.3節） | ○ | sema.rs `ensure_global` |
| `extern fn` の戻り値 `-> bool in C`・`-> Option<u8> in A`（有無はキャリー）・`-> u16 in AX`、`clobbers(A, X, Y, flags)`、レジスタの誤りは E0607（7.6節） | ○ | sema.rs `check_extern_param`・`check_extern_sig`、cgexpr.rs `call_fn` |
| 標準のインターフェース `sound`（`tone`・`off`。20.4節） | ○ | std/sound.ax65 |
| `ax65 new` のひな形を実機の v2（16.7節）に、`ax65 targets` に exit・irq・nmi・reserve を表示 | ○ | cli.rs |
| `ax65 explain` に E0607・E1110・E1111 を追加 | ○ | explain.rs |

`extern fn` の `clobbers` は検査だけで、コード生成では呼び出しの後は A・X・Y・フラグがすべて壊れたものとして扱います（保たれるレジスタを使う最適化は未実装）。
`extern fn` は常に 10進モードを解除した状態で呼ばれます（`sed` は BCD の演算の中だけで使い、すぐ `cld` するため）。

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
| 7 | 関数、const fn、comptime fn、asm fn、再帰の禁止、extern fn（1.1 の in C・Option<u8>・clobbers の名前）、#[inline] 系 | ○ | clobbers は検査だけ |
| 8 | モジュール、use（as・as _・{…}）、pub、interface/impl/bind、init の順序、使われないコードの除去 | ○ | |
| 9 | 属性 | ○ | |
| 10 | 割り込み処理（使うレジスタだけ退避）、BRK の振り分け、文脈の規則、irq_enable の規則、shared の規則、atomic（割り込みのないターゲットでは命令なし） | ○ | |
| 11 | I/O（in・out・rmw、読み捨ての bit、所有者、アドレスの重なり） | ○ | |
| 12 | cycles・max_cycles・delay・wait_interrupt・core::us | △ | cycles と max_cycles は分岐のない本体だけ。io_span は × |
| 13 | const の計算、comptime fn（int・real・数学関数）、表の生成、include_bin、comptime assert、target.*・build.*、comptime if/match | ○ | |
| 14 | asm ブロック・asm (in/out)・asm fn・unsafe asm・smc の検査 | ○ | ループの回数の上限の解析は × |
| 15 | 起動と終了（exit = rts・monitor・halt・never）、引数（A・X・Y と引数領域）、戻り値（キャリー・A・AX・AXY・結果領域）、スタックの予算 | ○ | |
| 16 | ターゲット定義（extends、region、reserve（足し算の継承、ベクタの自動の予約）、irq・nmi（none・hw・indirect・jump）、place const/code、encoding、bind、const、output ほか） | ○ | place は code だけを使う |
| 17 | プロジェクトファイル、モジュールの探し方、コマンド、オプション、出力形式、リスティング | △ | fmt は 16進の書き方だけ。--json は最小限 |
| 18 | エラー（E0101〜E1210。1.1 の E0607・E1110・E1111 を含む）、警告、メッセージの書式、レポート | △ | reversible_loop の助言、最も長い atomic のサイクル数は × |
| 19 | 最適化の段階 | △ | 下の表 |
| 20 | 組み込み関数、core（us・mul8・mul16・divmod8・divmod16・Nz8・Nz16・QR8・QR16）、bcd、標準のインターフェース（console・screen・keyboard・sound・timer・mul） | △ | text（数値の文字列化）は × |
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
- `extern fn` の `clobbers` に書かなかったレジスタが保たれることを、コード生成で利用していません（常に全部壊れたものとして扱う）。
- `exit = rts` のとき、モニタが積んだスタックの分は検査しません（`stack` にはプログラムが使える量を書く。16.4節）。
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
std/               標準ライブラリ（core、bcd、console、screen、keyboard、sound、timer、mul、mul_shift）
tests/             run_meteor.sh と meteor_compare.py（1.1 の sample/tools を使う）
```
