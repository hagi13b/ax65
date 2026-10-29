<a id="s15"></a>
## 15. 実行環境と呼び出し規約

<a id="s15-1"></a>
### 15.1 起動と終了

コンパイラが作る起動処理は、次の順序で実行されます。

1. 割り込みを止め（`sei`）、10進モードを解除し（`cld`）、スタックポインタを設定する
2. `static`・`shared` に初期値を設定する（[4.6節](#s4-6)）
3. 割り込みベクタを書く（ターゲット定義の `irq`・`nmi` が `indirect` か `jump` の場合）
4. 各モジュールの `init` を、読み込まれる側から順に実行する（[8.5節](#s8-5)）
5. `main` を呼ぶ（割り込みは止まったまま。[10.3節](#s10-3)）
6. `main` が終わったら、ターゲット定義の `exit` に従う

```asm
start:                          ; 読み込み先（load）の先頭。Wozmon から 300R で実行される
        sei                     ; 1. 割り込みを止める
        cld                     ; 10進モードを解除する
        ldx #$FF                ; スタックポインタの初期値
        txs                     ; 設定する
        lda #10                 ; 2. 初期値：PLAYER_X = 10
        sta PLAYER_X            ; 書く
        stz SCORE               ; 2. 初期値：SCORE = 0（下位）
        stz SCORE+1             ; 上位も 0 にする（SCORE は bcd16 で 2 バイト）
        lda #<on_irq            ; 3. IRQ ベクタ（indirect($0280)）の下位
        sta $0280               ; 書く
        lda #>on_irq            ; 上位
        sta $0281               ; 書く
        lda #$40                ; 4. init の中身（展開される）
        sta ACR                 ; 書く
        jsr main                ; 5. main を呼ぶ
        jmp $FF1F               ; 6. exit = monitor($FF1F)
```

<a id="s15-2"></a>
### 15.2 引数

```
fn draw(x: u8, y: u8, color: u8) { ... }    // A、X、Y で渡す
fn box(x: u8, y: u8, w: u8, h: u8) { ... }  // 4 つめの h は box の引数領域に直接書く
fn print(s: str) { ... }                    // ポインタは A（下位）と X（上位）で渡す
```

```asm
        lda #<TITLE             ; print(TITLE)：アドレスの下位を A に
        ldx #>TITLE             ; 上位を X に
        jsr print               ; 呼ぶ（print は入口でゼロページのポインタに書く）
```

- 引数は、呼び出し先が持つ固定アドレスの変数として扱います。
- 先頭から 3 バイト分は、A、X、Y の順に入れて渡します。u16 とポインタは A（下位）と X（上位）の 2 本を使います。呼び出し先は、後で使う値だけを自分の固定領域にしまいます。
- レジスタで受け取ったポインタは、呼び出し先が入口で自分のゼロページ領域に書きます。
- 4 バイトめ以降の引数（レジスタに入りきらないポインタを含む）は、呼び出し元が、呼び出し先専用の引数領域に直接書き込みます。
- `-O1` 以上では、関数ごとに渡し方を変えてもかまいません。ただし `extern fn` と、アセンブリから呼ばれる関数は、常にこの規則に従います。

関連 Q&A：[D-1](#qa-d1)

<a id="s15-3"></a>
### 15.3 戻り値

```
struct QR { q: u8, r: u8 }
fn div10(n: u8) -> QR { ... }           // 10 で割った商と余り

fn show(n: u8) {
    let d = div10(n);                   // 結果は div10 の結果領域にある（コピーしない）
    put_digit(d.q);                     // div10 の結果領域を直接読む
    put_digit(d.r);
}
```

返し方は、戻り値の型の大きさで決まります。

| 戻り値の型 | 返し方 |
|---|---|
| なし | 何も返さない |
| `bool` | キャリー |
| 1 バイトの型（u8、i8、char、bcd8、範囲型、列挙型、8 ビット以下のビットフラグ、`fnset` の値） | A |
| 2 バイトの型（u16、i16、ポインタ、`str`・`zstr`・`histr`、9〜16 ビットのビットフラグ、bcd16） | A（下位）と X（上位） |
| 3 バイトの型（u24、i24） | A（下位）、X、Y（上位） |
| 4 バイト以上の型（u32、i32、bcd32）、構造体、`strbuf<N>` | 呼び出し先の結果領域 |
| `Option<T>` | 有無はキャリー、値は上の規則（`Option<bool>` の値は A に 0 か 1） |

- 結果領域は固定アドレスで、呼び出し元はコピーせずに直接読みます。同じ関数をもう一度呼ぶまで値は変わりません。古い結果を使い終わる前に同じ関数を呼ぶ場合だけ、コンパイラが自動でコピーを入れ、レポートに出します（[18.4節](#s18-4)）。構造体のフィールドの数に制限はありません。
- 配列は返せません（エラー）。返したい場合は構造体に入れます。

関連 Q&A：[D-5](#qa-d5)、[D-6](#qa-d6)、[D-7](#qa-d7)、[M-15](#qa-m15)

<a id="s15-4"></a>
### 15.4 レジスタ

```
fn rand() -> u8 { ... }                 // A とフラグだけを壊す（X、Y は保たれる）

fn spawn_all() {
    for m in Mid {                      // m は X に置ける：rand が X を壊さないとコンパイラが知っている
        METEORS[m].x = rand() & 15;
    }
}
```

- 関数を呼ぶと A、X、Y は壊れる可能性がある、という前提にします。ただし、コンパイラはプログラム全体を解析し、呼び出し先が実際に壊すレジスタだけを、呼び出し元で退避します。
- 呼び出し先が X を壊さなければ、呼び出し元はループの番号を X に置いたまま呼べます。

<a id="s15-5"></a>
### 15.5 スタックの予算

```
エラー[E0301]：スタック予算超過（必要 140 バイト / 予算 128 バイト）
  メイン最深経路：main → update → draw_sprite → plot（112 バイト）
  IRQ：on_irq → read_kbd（16 バイト）
  NMI：on_nmi（12 バイト）
```

- スタックに積まれるのは、`jsr` の戻り番地（2 バイト）、割り込み時に CPU が積む戻り番地とフラグ（3 バイト）、割り込み処理の入口でのレジスタの退避、コンパイラが入れる一時的な退避と `atomic` の `php` だけです。ローカル変数は積みません。
- コンパイラは、メイン・IRQ 処理・NMI 処理それぞれのすべての呼び出し経路について最大の使用量を求め、合計がターゲット定義の `stack` を超えたら、原因の経路を示してエラーにします。

関連 Q&A：[D-3](#qa-d3)、[A-1](#qa-a1)

<a id="s15-6"></a>
### 15.6 CPU による違い

65C02 では、次の追加命令を活用します。

| 追加命令 | 主な用途 |
|---|---|
| `(zp)` | ポインタの指す先の読み書き（Y を使わない） |
| `phx` `phy` `plx` `ply` | 割り込み処理のレジスタ退避 |
| `inc a` `dec a` | A の ±1 |
| `stz` | 0 の書き込み |
| `bra` | 無条件の短い分岐 |
| `jmp (abs,x)` | ジャンプテーブル（`fnset`、`match`） |
| `tsb` `trb` | ビットの操作 |
| `wai`（W65C02S のみ） | 割り込み待ち |
| `rmb` `smb` `bbr` `bbs`（W65C02S など一部の品種のみ） | ゼロページのビット操作（品種をターゲット定義で区別する） |

```asm
; 割り込み処理の入口（65C02 のターゲット）
        pha                     ; A を退避する
        phx                     ; X を直接退避する（65C02 の命令）
; 同じ処理（NMOS 6502 のターゲット）
        pha                     ; A を退避する
        txa                     ; X を A に移す（phx がないため）
        pha                     ; 退避する
        cld                     ; NMOS は割り込み時に 10進モードを解除しないので、自分で解除する
```

NMOS 6502 のターゲットでは、コンパイラが次の点を自動で処理します。

- 割り込み処理の入口に `cld` を入れる（NMOS は割り込み時に 10進モードを解除しない）
- 間接ジャンプの格納先を $xxFF に置かない（NMOS の `jmp (ind)` は、$xxFF から読むと上位バイトを $xx00 から読む）
- 10進演算の後の N・Z フラグを判定に使わない
- 65C02 の追加命令を、同じ働きの命令の組み合わせに置き換える

未定義命令は、どの CPU でも使いません。

---

<a id="s16"></a>
## 16. ターゲット定義

<a id="s16-1"></a>
### 16.1 ファイルと継承

- ターゲット定義は、`<ターゲット名>.target` というファイルに、AX65 と同じ文法で書きます。プロジェクトファイルの `target_dir`（既定は `targets`）から探します。
- `target 名前 extends 親 { ... }` で、親の設定を引き継いで差分だけを書けます。子で書いた項目は親の項目を置き換えます（`region` は同じ種類ごとに置き換え、`bind`・`const`・`encoding` は名前ごとに置き換え）。

```
// targets/v3.target
target v3 extends v2 {
    clock = 2_000_000;                  // v2 の 1MHz を置き換える
    bind screen = hw::v3_screen;        // v2 にない bind を足す
}
```

<a id="s16-2"></a>
### 16.2 項目の一覧

```
// 最小のターゲット定義（必須の項目だけ）
target mini {
    cpu   = Cpu::W65C02S;
    clock = 1_000_000;
    stack = 64;
    run   = ram;
    load  = $0200;
    exit  = halt;
    region zp    = $00..=$FF;
    region stack = $0100..=$01FF;
    region ram   = $0200..=$0FFF;
}
```

| 項目 | 書き方 | 必須 | 意味 |
|---|---|---|---|
| `cpu` | `Cpu::W65C02S` など | ○ | CPU の品種（`Cpu::Nmos6502`、`Cpu::C65C02`、`Cpu::W65C02S`、`Cpu::R65C02`） |
| `clock` | `1_000_000` | ○ | クロック周波数（Hz）。`core::us()` に使う |
| `stack` | `128` | ○ | ハードウェアスタックの予算（バイト。[15.5節](#s15-5)） |
| `run` | `ram` または `rom` | ○ | RAM に読み込んで実行するか、ROM から実行するか |
| `load` | `$0300` | `run = ram` のとき | 読み込み先のアドレス（起動処理の先頭） |
| `exit` | `monitor($FF1F)`、`halt`、`never` | ○ | `main` が終わったときの動作 |
| `irq`、`nmi` | `hw`、`indirect($0280)`、`jump($0280)` | 割り込みを使うとき | 割り込みベクタの置き方 |
| `irq_interval` | `16_666` | | IRQ の最短の間隔（サイクル）。割り込み処理の時間の検査に使う |
| `region` | `region ram = $0200..=$1FFF;` | ○ | メモリの区画（[16.3節](#s16-3)） |
| `reserve` | `reserve $24..=$2B;` | | 使用禁止の範囲 |
| `place` | `place const in $F000..=$F7FF;` | | 種類ごとの置き場所の指定 |
| `encoding` | `encoding screen { ... }` | | 文字の変換表（[16.5節](#s16-5)） |
| `bind` | `bind console = hw::acia_console;` | | インターフェースの実装（[16.6節](#s16-6)） |
| `const` | `const SCREEN_WIDTH: u8 = 40;` | | コードを伴わない性質（`target.SCREEN_WIDTH` で読む） |
| `output` | `output = [woz, bin, sym, lst];` | | 既定で出力する形式（[17.6節](#s17-6)） |

<a id="s16-3"></a>
### 16.3 メモリの区画

```
region zp    = $00..=$EF;               // ゼロページのうち使ってよい範囲
region stack = $0100..=$01FF;
region ram   = [$0200..=$03FF, $0800..=$BFFF];   // 複数の範囲を並べられる
region io    = [$0400..=$07FF, $6000..=$8FFF];
region rom   = $E000..=$FFFF;
reserve $24..=$2B;                      // Wozmon が使うゼロページ
place const in $F000..=$F7FF;           // 定数だけを ROM のこの範囲に置く
```

- 区画の種類は `zp`、`stack`、`ram`、`io`、`rom` です。範囲は `..=`（終わりを含む）か `..`（含まない）で書きます。
- `@` で指定したアドレスが区画の外や `reserve` の中ならエラーです。I/O 変数のアドレスは `io` の区画の中でなければなりません。
- コンパイラが作るものは、コード、定数、初期値の元データ、変数の 4 種類です。置き場所は次のとおりです（`place` で変えられる）。

| | コード・定数・初期値の元データ | 変数 |
|---|---|---|
| `run = ram` | `ram` の区画（`load` のアドレスから） | `ram` の区画（コードの後ろ） |
| `run = rom` | `rom` の区画 | `ram` の区画 |

- 区画ごとの使用量をレポートに出します。

<a id="s16-4"></a>
### 16.4 起動・終了・割り込みベクタ

```
// RAM で実行し、終わったらモニタへ戻る（モニタが IRQ を $0280 経由で渡す）
run  = ram;  load = $0300;  exit = monitor($FF1F);  irq = indirect($0280);
// ROM から起動し、終わらない（リセットベクタと IRQ ベクタを ROM に直接書く）
run  = rom;  exit = never;  irq = hw;  nmi = hw;
```

| `exit` の指定 | 動作 |
|---|---|
| `monitor($FF1F)` | モニタの入口へ `jmp` する |
| `halt` | その場で止まる（無限ループ） |
| `never` | `main` は終わってはならない。終わりうる場合はエラー |

| `irq`・`nmi` の指定 | 意味 |
|---|---|
| `hw` | ROM のベクタ（$FFFA、$FFFE）に直接書く（`run = rom` のとき） |
| `indirect($0280)` | ROM が `jmp ($0280)` で飛ぶ形。$0280 に処理のアドレスを書く |
| `jump($0280)` | ROM が `jmp $0280` で飛ぶ形。$0280 に `jmp` 命令ごと書く |

- `run = rom` では、リセットベクタ（$FFFC）に起動処理のアドレスを書きます。
- `indirect` と `jump` は起動処理の中で、割り込みを止めた状態で書き込みます（[15.1節](#s15-1)）。

関連 Q&A：[C-2](#qa-c2)

<a id="s16-5"></a>
### 16.5 文字の変換表

```
encoding screen {
    ' '..='?' => $A0..=$BF;             // 文字の範囲 → 値の範囲
    '@'..='_' => $C0..=$DF;
    '\r'      => $8D;                   // 1 文字 → 1 値
}

encoding seg7 {                         // 文字以外のバイトにも使える（7セグメント LED）
    '0' => $3F;  '1' => $06;  '2' => $5B;  '3' => $4F;  '4' => $66;
    '5' => $6D;  '6' => $7D;  '7' => $07;  '8' => $7F;  '9' => $6F;
}
```

- 項目は「1 文字 => 1 値」か「文字の範囲 => 値の範囲」です。範囲の両側の個数が違う場合はエラーです。
- 同じ文字を 2 回書くとエラーです。変換表にない文字をソースで使うとエラーです。
- ソースでは `'A'@screen`、`p"SCORE"@screen` のように使います（[3.3節](#s3-3)）。

関連 Q&A：[G-2](#qa-g2)、[G-3](#qa-g3)

<a id="s16-6"></a>
### 16.6 bind と定数

```
bind console = hw::acia_console;        // console インターフェースの実装
bind screen  = hw::v3_screen;
const SCREEN_WIDTH: u8 = 40;
const HAS_EXT_RAM: bool = true;
```

```
// ソースの側
comptime if target.has(screen) { screen::clear(); }
comptime if target.HAS_EXT_RAM { use_big_buffer(); }
static LINE: [u8; target.SCREEN_WIDTH] = [0; target.SCREEN_WIDTH];
```

- 機能が「ある」とは、その機能のインターフェースに実装が bind されていることです。`target.has(screen)` で調べます。機能の一覧は書きません。
- コードを伴わない性質（画面の幅、拡張 RAM の有無など）は、名前付きの定数で書き、`target.名前` で読みます。
- コマンドラインの `--bind` で、bind を一時的に差し替えられます（[17.5節](#s17-5)）。

関連 Q&A：[H-1](#qa-h1)、[H-2](#qa-h2)

<a id="s16-7"></a>
### 16.7 ターゲット定義の例

KIM-1 級から始めて、シリアル端末、画面と育てていく例です。

```
// targets/kim.target — 最初の構成（RAM 1KB、7セグメント LED）
target kim {
    cpu   = Cpu::W65C02S;
    clock = 1_000_000;
    stack = 64;
    run   = ram;
    load  = $0200;
    exit  = halt;
    region zp    = $00..=$EF;
    region stack = $0100..=$01FF;
    region ram   = $0200..=$03FF;
    region io    = $1700..=$17FF;
    encoding seg7 { '0' => $3F; '1' => $06; '2' => $5B; '3' => $4F; '4' => $66;
                    '5' => $6D; '6' => $7D; '7' => $07; '8' => $7F; '9' => $6F; }
    bind console = hw::led_console;
    const LED_DIGITS: u8 = 6;
    output = [ptp, lst];
}

// targets/v2.target — シリアル端末と 8KB の RAM
target v2 {
    cpu          = Cpu::W65C02S;
    clock        = 1_000_000;
    stack        = 128;
    run          = ram;
    load         = $0300;
    exit         = monitor($FF1F);
    irq          = indirect($0280);
    nmi          = indirect($0282);
    irq_interval = 16_666;
    region zp    = $00..=$EF;
    region stack = $0100..=$01FF;
    region ram   = $0200..=$1FFF;
    region io    = $6000..=$8FFF;
    reserve $24..=$2B;
    reserve $0200..=$0283;
    output = [woz, bin, sym, lst];
    bind console = hw::acia_console;
}

// targets/v3.target — v2 に画面を追加
target v3 extends v2 {
    clock        = 2_000_000;
    irq_interval = 33_333;
    load         = $0800;
    region ram = [$0200..=$03FF, $0800..=$BFFF];
    region io  = [$0400..=$07FF, $6000..=$8FFF];
    encoding screen { ' '..='?' => $A0..=$BF; '@'..='_' => $C0..=$DF; }
    bind screen = hw::v3_screen;
    const SCREEN_WIDTH: u8 = 40;
}

// targets/romkit.target — ROM に焼いて電源投入で起動する構成
target romkit {
    cpu   = Cpu::W65C02S;
    clock = 4_000_000;
    stack = 128;
    run   = rom;
    exit  = never;
    irq   = hw;
    nmi   = hw;
    region zp    = $00..=$FF;
    region stack = $0100..=$01FF;
    region ram   = $0200..=$3FFF;
    region io    = $6000..=$7FFF;
    region rom   = $8000..=$FFFF;
    output = [rom, hex, sym, lst];
    bind console = hw::acia_console;
}
```

---

<a id="s17"></a>
## 17. プロジェクトとコマンドライン

<a id="s17-1"></a>
### 17.1 フォルダ構造

```
<ax65 のインストール先>/
├── bin/ax65                     コンパイラ
└── std/                         標準ライブラリ（17.3節の探す順の最後）
    ├── core.ax65                us()、mul8、divmod8、Nz8 など（20.2節）
    ├── bcd.ax65  text.ax65      20.3節
    ├── console.ax65             インターフェース（20.4節）
    ├── screen.ax65  keyboard.ax65  timer.ax65  mul.ax65
    ├── mul_shift.ax65           mul の標準の実装（bind しなければこれを使う）
    ├── mul_table.ax65           mul の表を使う実装
    └── ps2_keyboard.ax65 ...    よく使う実装

meteor/                          プロジェクト（『サンプルと効果測定 1.0』のサンプル）
├── ax65.proj                    プロジェクトファイル（17.2節）
├── targets/                     ターゲット定義（16章）
│   ├── v2.target
│   └── v3.target
├── src/
│   └── meteor.ax65              main を含むファイル
├── hw/                          自分のマシン用のインターフェースの実装
│   ├── acia_console.ax65        impl console
│   └── v3_screen.ax65           impl screen
├── data/                        include_bin で読むファイル（必要なら）
└── build/                       出力（ax65 build が作る）
    ├── v2/
    │   ├── meteor.woz  meteor.bin  meteor.sym  meteor.lst
    │   └── meteor.report
    └── v3/
        └── ...
```

<a id="s17-2"></a>
### 17.2 プロジェクトファイル

プロジェクトのフォルダの `ax65.proj` に書きます。

```
// ax65.proj
project meteor {
    main       = "src/meteor.ax65";     // main を含むファイル
    paths      = ["."];                 // use でモジュールを探す場所
    target_dir = "targets";             // ターゲット定義を探す場所
    targets    = [v2, v3];              // ax65 check --all-targets の対象
    default    = v2;                    // -t を省略したときのターゲット
    opt        = O2;                    // -O を省略したときの最適化の段階
    out_dir    = "build";               // 出力先（<out_dir>/<ターゲット名>/）

    lints {                             // 警告の扱い（allow / warn / deny）
        naming         = warn;
        unused_imports = warn;
    }
    fmt {                               // ax65 fmt の書き方
        hex   = dollar;                 // 16進：dollar（$FF）または c（0xFF）
        array = rust;                   // 配列の型：rust（[T; N]）または legacy（T[N]）
    }
}
```

| 項目 | 既定値 | 意味 |
|---|---|---|
| `main` | （必須） | `main` を含むファイル |
| `paths` | `["."]` | `use` でモジュールを探す場所（並べた順に探す） |
| `target_dir` | `"targets"` | ターゲット定義のフォルダ |
| `targets` | `target_dir` の全ファイル | `--all-targets` の対象 |
| `default` | （1 つしかなければそれ） | 既定のターゲット |
| `opt` | `O2` | 既定の最適化の段階（`O0`、`O1`、`O2`、`Os`） |
| `out_dir` | `"build"` | 出力先 |
| `step_limit` | `1_000_000` | コンパイル時の計算の手順数の上限 |
| `lints { 名前 = allow / warn / deny; }` | すべて `warn` | 警告ごとの扱い（[18.2節](#s18-2)） |
| `fmt { hex = …; array = …; }` | `dollar`、`rust` | `ax65 fmt` の書き方 |

<a id="s17-3"></a>
### 17.3 モジュールの探し方

```
// ax65.proj：paths = ["."]
use console;                            // ./console.ax65 がなければ <ax65>/std/console.ax65
use hw::via;                            // ./hw/via.ax65
bind console = hw::acia_console;        // ./hw/acia_console.ax65（ターゲット定義の中）
bind keyboard = ps2_keyboard;           // ./ps2_keyboard.ax65 がなければ <ax65>/std/ps2_keyboard.ax65
```

`use hw::acia_console;` や `bind console = hw::acia_console;` のモジュールは、次の順に探します。最初に見つかったものを使い、見つからなければエラーです。

1. プロジェクトファイルの `paths` に並べたフォルダ（`<path>/hw/acia_console.ax65`）
2. 標準ライブラリ（`<ax65>/std/`）

`use console;` のように、プロジェクトに同じ名前のファイルがなければ、標準ライブラリのインターフェースが使われます。標準ライブラリのモジュールは、前に何も付けず、名前だけで書きます（`use core::mul8;`、`bind mul = mul_table;`）。

関連 Q&A：[M-12](#qa-m12)

<a id="s17-4"></a>
### 17.4 コマンド

| コマンド | 意味 |
|---|---|
| `ax65 build` | コンパイルして出力する |
| `ax65 check` | コンパイルの検査だけを行い、出力はしない（`--all-targets`、`--all-levels` で範囲を広げる） |
| `ax65 fmt` | ソースの書き方をそろえる（`fmt` の設定に従う） |
| `ax65 explain <番号>` | エラー・警告の詳しい説明と例を表示する |
| `ax65 new <名前>` | プロジェクトのひな形（ax65.proj、targets/、src/）を作る |
| `ax65 targets` | ターゲット定義の一覧と、継承の関係、bind の一覧を表示する |
| `ax65 --version`、`ax65 --help` | 版、使い方を表示する |

```
ax65 build                                   # 既定のターゲット（default）、既定の段階（opt）
ax65 build -t v3                             # ターゲットを指定
ax65 build -t v2 -O0                         # 最適化をしない
ax65 build -t v2 -Os --emit bin,lst          # 速さ優先、出力形式を指定
ax65 build -t v2 -D debug -D level=3         # build.debug = true、build.level = 3
ax65 build -t v2 --bind console=sim::console # bind を一時的に差し替える
ax65 build -t v2 --json > result.json        # メッセージとレポートを JSON で
ax65 check --all-targets --deny-warnings     # すべてのターゲットで、警告もエラーとして検査
ax65 check --all-levels                      # すべての最適化の段階で、資源とタイミングを検査
ax65 fmt --check                             # 書き方がそろっているかだけを調べる
ax65 explain E0412
```

<a id="s17-5"></a>
### 17.5 オプション

```
ax65 build -t v2 -O2 -D debug --emit woz,lst -o out/ --woz-run
ax65 build -t v2 -A naming --deny unused_variables   # naming を止め、unused_variables をエラーにする
ax65 check --all-targets --all-levels --deny-warnings --json
```

| オプション | 対象のコマンド | 意味 |
|---|---|---|
| `-t <名前>`、`--target <名前>` | build、check | ターゲットを指定する |
| `--all-targets` | check | プロジェクトのすべてのターゲットで検査する（[13.6節](#s13-6)） |
| `--all-levels` | check | すべての最適化の段階（`-O0`〜`-Os`）で検査する（[19.1節](#s19-1)） |
| `-O0`、`-O1`、`-O2`、`-Os` | build、check | 最適化の段階（[19章](#s19)） |
| `-D <名前>`、`-D <名前>=<値>` | build、check | `build.名前` の値を決める（値を省略すると `true`。数値か文字列） |
| `--bind <インターフェース>=<モジュール>` | build、check | bind を一時的に差し替える |
| `--emit <形式>,...` | build | 出力形式（[17.6節](#s17-6)）。ターゲット定義の `output` より優先 |
| `-o <フォルダ>`、`--out-dir <フォルダ>` | build | 出力先 |
| `--project <ファイル>` | すべて | プロジェクトファイルを指定する（既定は現在のフォルダの `ax65.proj`） |
| `--deny-warnings` | build、check | すべての警告をエラーとして扱う |
| `-A <名前>`、`-W <名前>`、`--deny <名前>` | build、check | 警告ごとの扱いを allow／warn／deny にする（[18.2節](#s18-2)） |
| `--json` | build、check | メッセージとレポートを JSON で出力する |
| `--no-report` | build | レポートを出力しない |
| `--lst-plain` | build | リスティングの命令の説明を省く |
| `--woz-line <n>` | build | Wozmon 形式の 1 行のバイト数（既定 8） |
| `--woz-run` | build | Wozmon 形式の最後に実行の命令（`300R` など）を付ける |
| `--step-limit <n>` | build、check | コンパイル時の計算の手順数の上限 |
| `--check` | fmt | 書き換えずに、そろっていなければエラー |
| `-v`、`--verbose` | すべて | 詳しい経過を表示する |

関連 Q&A：[M-16](#qa-m16)

<a id="s17-6"></a>
### 17.6 出力形式

| 形式（`--emit` の名前） | 拡張子 | 用途 |
|---|---|---|
| `bin` | `.bin` | 読み込み先から始まる生のバイト列 |
| `woz` | `.woz` | `0300: A9 00 ...` の 16進テキスト。シリアル端末から Wozmon に貼り付ける |
| `hex` | `.hex` | Intel HEX。ROM ライタ向け |
| `rom` | `.rom` | ROM の大きさちょうどのイメージ。空きは $FF、ベクタを含む |
| `ptp` | `.ptp` | KIM-1 の紙テープ形式 |
| `sym` | `.sym` | 関数・変数・ラベルのアドレスの一覧（エミュレータやデバッガ向け） |
| `lst` | `.lst` | リスティング（[17.7節](#s17-7)） |
| `report` | `.report` | レポート（[18.4節](#s18-4)。`--no-report` がなければ常に出力） |

```
0300: 78 D8 A2 FF 9A A9 0A 85
0308: 30 A9 5A 85 33 64 31 64
...
300R
```

<a id="s17-7"></a>
### 17.7 リスティング

```
                         ; meteor.ax65:155   METEORS[m].y += 1;
0427  F6 3E       6      inc M_Y,x       ; METEORS[m].y を 1 増やす
                         ; meteor.ax65:156   if METEORS[m].y == FIELD_H {
0429  B5 3E       4      lda M_Y,x       ; METEORS[m].y を読む
042B  C9 10       2      cmp #16         ; FIELD_H と比べる
042D  D0 1A       2/3    bne upd_chk     ; 違えば else へ
```

- ソースの各行と、生成した命令のアドレス・バイト列・サイクル数（分岐は「飛ばない/飛ぶ」、`*` はページ境界をまたぐと +1）・命令・命令の説明を並べます。
- 命令の説明は `--lst-plain` で省けます。

関連 Q&A：[I-1](#qa-i1)

---

