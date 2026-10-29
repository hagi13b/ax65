<a id="s7"></a>
## 7. 関数

<a id="s7-1"></a>
### 7.1 関数の定義

```
fn damage(e: Eid, n: u8) -> bool {
    ENEMIES[e].hp = ENEMIES[e].hp.saturating_sub(n);
    ENEMIES[e].hp == 0                  // 最後の式が戻り値
}

fn reset() {                            // 引数も戻り値もない関数（サブルーチン）
    SCORE = 0;
}

fn clamp(v: u8, hi: u8) -> u8 {         // 引数は関数の中で書き換えられる
    if v > hi { v = hi; }
    v
}
```

- 関数とサブルーチンを区別せず、「入口と出口のデータを宣言したサブルーチン」として 1 つの仕組みにまとめます。
- 引数は、関数の中で書き換えられます（呼び出し元の値は変わらない）。`mut` は書きません。
- 戻り値は 1 つです。複数の値は構造体にまとめて返します（[15.3節](#s15-3)）。
- 引数と戻り値の渡し方は [15.2節](#s15-2)、[15.3節](#s15-3) にあります。

関連 Q&A：[D-2](#qa-d2)

<a id="s7-2"></a>
### 7.2 const fn

```
const fn clamp(v: u8, lo: u8, hi: u8) -> u8 {
    if v < lo { lo } else if v > hi { hi } else { v }
}

const LIMIT: u8 = clamp(250, 10, 200);  // コンパイル時に計算される
fn update(x: u8) { let y = clamp(x, 10, 200); }   // 実行時にも呼べる

const fn bad() -> u8 { PLAYER_X }      // エラー：const fn の中で static を読んでいる
```

- `const fn` は、コンパイル時に計算でき、実行時にも呼べる関数です（Rust の `const fn` と同じ）。
- `const fn` の中では、`static`・`shared`・I/O に触れられず、`const fn` 以外の関数を呼べません。条件を満たさなければエラーです。`const` の値は読めます。
- どちらで計算しても、型の規則（あふれは一周して戻る、`as` は下位のバイトを取るなど。[3.14節](#s3-14)）は同じで、結果は一致します。
- コンパイル時にしか呼べない関数は `comptime fn` です（[7.3節](#s7-3)）。

関連 Q&A：[K-6](#qa-k6)、[K-17](#qa-k17)

<a id="s7-3"></a>
### 7.3 comptime fn

```
comptime fn sine_at(i: int) -> i8 {
    let r: real = sin(i as real * 2.0 * PI / 256.0);
    round(r * 127.0) as i8              // 範囲外ならコンパイルエラー
}
```

- コンパイル時にしか呼べない関数です。詳しくは [13.2節](#s13-2) にあります。

<a id="s7-4"></a>
### 7.4 asm fn

```
asm fn rand_step(r: u8) -> u8 {         // r は A で届き、A で返す（標準の呼び出し規約）
    asl a                   ; 左に1ビットずらす（はみ出たビットはキャリーへ）
    bcc done                ; はみ出たビットが 0 ならそのまま
    eor #$1D                ; 1 なら決まった値と XOR を取る
done:                       ; 合流地点
    rts                     ; A を戻り値として返す
}
```

- 関数全体をアセンブリで書きます。引数と戻り値は標準の呼び出し規約（[15.2節](#s15-2)、[15.3節](#s15-3)）に従います。
- コンパイラは中身を解析し、壊すレジスタ、スタックの使用量、サイクル数を自分で求めます（[14章](#s14)）。

<a id="s7-5"></a>
### 7.5 再帰の禁止

```
fn a() { b(); }
fn b() { a(); }                         // エラー：a → b → a の再帰
```

- 直接・間接を問わず、呼び出しグラフに輪ができたらエラーです。`fnset` を通した呼び出しも含めて検査します。

関連 Q&A：[A-1](#qa-a1)

<a id="s7-6"></a>
### 7.6 extern fn

```
extern fn rom_print(c: char in A)       // 引数 c を A で渡す（Wozmon の ECHO）
    @ $FFEF                             // ルーチンのアドレス
    clobbers(flags)                     // 壊すもの（ここにないレジスタは保たれる）
    stack(2);                           // 使うスタック（戻り番地を含む）

fn hello() { rom_print('H'); rom_print('I'); }
```

- コンパイラが中身を解析できない ROM ルーチンについて、引数のレジスタ、アドレス、壊すもの、スタックの使用量を宣言します。
- 引数は `名前: 型 in A`（`A`、`X`、`Y`、`AX`）、戻り値は `-> u8 in A` のように書きます。`in` によるレジスタの指定は、`extern fn` の宣言にだけ書けます。
- 宣言の正しさはプログラマの責任です（Rust の `unsafe` と同じ位置づけ）。`extern fn` の宣言は、レポートの unsafe の項目に一覧で出ます（[18.4節](#s18-4)）。

関連 Q&A：[D-4](#qa-d4)

<a id="s7-7"></a>
### 7.7 インライン化の指定

```
#[inline]
fn sq(x: u8) -> u8 { core::mul8(x, x) as u8 }

#[inline(never)]
fn debug_dump() { ... }

#[cold]
fn respawn(m: Mid) { ... }              // めったに呼ばれない（-Os でも展開しない）
```

| 属性 | 意味 |
|---|---|
| `#[inline]` | 展開してよい。`-O0` でも展開する |
| `#[inline(always)]` | 必ず展開する。展開できない場合はエラー |
| `#[inline(never)]` | 決して展開しない |
| `#[cold]` | めったに呼ばれない。`-Os` でも展開せず、何度も回るループの外に置く |

- 属性がない関数を展開するかどうかは、最適化の段階で決まります（[19章](#s19)）。属性は段階による判断より優先されます。
- 展開できない関数は、途中に `rts` がある `asm fn`、`unsafe asm` を含む関数、`extern fn`、割り込み処理の関数です。
- 末尾呼び出し（`jsr` の直後が `rts`）は、段階にかかわらず `jmp` に置き換えます。
- どの関数を展開したか、しなかったか、その理由をレポートに出します。

関連 Q&A：[I-2](#qa-i2)

---

<a id="s8"></a>
## 8. モジュール

<a id="s8-1"></a>
### 8.1 ファイルとモジュール

```
meteor/
├── src/meteor.ax65          モジュール meteor（main を含む）
└── hw/
    ├── acia_console.ax65    モジュール hw::acia_console
    └── via.ax65             モジュール hw::via
```

- 1 つのファイルが 1 つのモジュールです。モジュールの名前はファイルの名前で、フォルダはパスの区切りになります（`hw/acia_console.ax65` は `hw::acia_console`）。
- 分割コンパイルとリンクは行いません。コンパイラは `use` をたどってすべてのファイルを読み込み、プログラム全体を 1 つとして解析します。
- モジュールの探し方は [17.3節](#s17-3) にあります。

<a id="s8-2"></a>
### 8.2 use

```
use math;                               // math.ax65 を読み込む。中の名前は math::sqrt() のように使う
use hw::via;                            // hw/via.ax65 を読み込む
use console::{putc, poll};              // putc と poll を、名前だけで使えるようにする
use hw::via as v;                       // 別名を付ける（v::init_ports()）
use hw::via as _;                       // init のためだけに読み込む（8.5節）
```

- `use` がファイルの読み込みを兼ねます（Rust の `mod` の宣言は不要）。
- 他のモジュールの名前は `モジュール名::名前` で参照します。
- `use` で取り込んだ名前が重なったらエラーです。
- モジュールの循環は許しますが、`init` の順序が決まらない場合はエラーです。

<a id="s8-3"></a>
### 8.3 公開範囲

```
pub fn putc(c: char) { ... }            // 他のモジュールから使える
fn put_raw(b: u8) { ... }               // このモジュールの中だけで使える
pub const WIDTH: u8 = 40;
```

- 宣言は既定で非公開です。`pub` を付けたものだけを他のモジュールから使えます。
- インターフェースの実装（`impl`）の中の、インターフェースの関数と定数は、自動で公開されます。`pub` は書きません（[8.4節](#s8-4)）。
- I/O 変数には `pub` を付けられません（[11.4節](#s11-4)）。

<a id="s8-4"></a>
### 8.4 インターフェースと実装

```
// <ax65>/std/console.ax65（標準ライブラリのインターフェース）
interface console {
    fn putc(c: char);
    fn getc() -> char;
    fn poll() -> Option<char>;
}

// hw/acia_console.ax65（ACIA による実装）
impl console {
    io      DATA:   u8         @ $8000;
    io(in)  STATUS: AciaStatus @ $8001;
    fn putc(c: char) {                  // インターフェースの関数：自動で公開される
        wait_tdre();
        DATA = c as u8;
    }
    fn poll() -> Option<char> {
        if STATUS.contains(AciaStatus::Rdrf) { Some(DATA as char) } else { None }
    }
    fn getc() -> char { loop { if let Some(c) = poll() { return c; } } }
    fn wait_tdre() {                    // インターフェースにない関数：このモジュールの中だけ
        while !STATUS.contains(AciaStatus::Tdre) { }
    }
    pub fn flush() { }                  // エラー：impl の中に pub は書けない
}

// hw/wozmon_console.ax65（ROM ルーチンを実装にする例）
impl console {
    extern fn putc(c: char in A) @ $FFEF clobbers(flags) stack(2);
    ...
}

// src/main.ax65（使う側：実装の名前は書かない）
use console;
fn main() { console::putc('A'); }

// targets/v2.target（どの実装を使うかを選ぶ）
bind console = hw::acia_console;
```

- インターフェースは関数と定数の形だけを宣言します。実装は `impl` で書き、どの実装を使うかはターゲット定義の `bind` で選びます（[16.6節](#s16-6)）。
- インターフェースはモジュールに対する仕組みで、Rust のトレイト（型に対する仕組み）とは別のものです。1 つのターゲットで、1 つのインターフェースに bind できる実装は 1 つだけです。
- `impl` の中の、インターフェースの関数と定数は自動で公開されます。`pub` は書きません（書くとエラー）。それ以外の宣言は、そのモジュールの中だけで使える補助の関数・変数です。
- 実装の過不足や型の違いはエラーです。
- 実装はコンパイル時に決まり、呼び出しは直接の `jsr` になります。
- bind されていないインターフェースも `use` できます。その場合、インターフェースの宣言は構文だけを検査し、`comptime if target.has(...)` の外で使うとエラーです。標準のインターフェース `mul` だけは、bind しなければ `mul_shift` が使われます（[20.4節](#s20-4)）。
- 使い分け：どの機種にもあり中身が違う機能はインターフェース、機種によって有無が違う機能や小さな違いは `comptime if`（[13.6節](#s13-6)）を使います。

関連 Q&A：[H-1](#qa-h1)、[H-2](#qa-h2)、[M-11](#qa-m11)

<a id="s8-5"></a>
### 8.5 init

```
// hw/via.ax65
io(out) DDRB: u8 @ $6002;
init {
    DDRB = $FF;                         // 起動時に main の前に 1 回だけ実行される
}

// src/main.ax65
use hw::via as _;                       // via の関数は使わないが、init は実行したい
```

- モジュールごとに `init` ブロックを書けます。起動時の順序は [15.1節](#s15-1) のとおりで、各モジュールの `init` は、読み込まれる側が先に実行されます。
- `init` は起動の手順の中に直接展開されます。`init` がなければコストは 0 です。
- `init` の実行中は割り込みが止まっています。
- `use` されたモジュールの `init` は、そのモジュールを使っていなくても必ず実行します。`init` を実行させない方法はありません。
- 使っていないモジュールの `use` には警告（`unused_imports`）を出します。`init` があるモジュールでは、起動時に実行されることも知らせます。`init` だけが目的の場合は `use ... as _;` と書くと警告が出ません。
- `unused_imports` は、取り込んだ名前ごとに出します（`use core::{mul8, mul16};` で `mul16` を使わなければ、`mul16` について警告）。
- すべての `init` の実行順、サイクル数、バイト数をレポートに出します。

関連 Q&A：[H-3](#qa-h3)、[H-4](#qa-h4)

<a id="s8-6"></a>
### 8.6 使われないコードの除去

```
use core::mul8;
fn area(w: u8, h: u8) -> u16 { mul8(w, h) }   // core の mul16、divmod8 などはプログラムに含まれない
```

- `main` と割り込み処理から呼び出しグラフをたどり、使われない関数・変数・表はプログラムに含めません。標準ライブラリを大きくしても、使った分しかメモリを使いません。
- モジュールごとのメモリ使用量（コード、変数、ゼロページ）をレポートに出します。

---

<a id="s9"></a>
## 9. 属性

属性は `#[名前]` または `#[名前(引数)]` と書き、直後の宣言・文に付きます。

```
#[zp] static X: u8 = 0;
#[interrupt(irq)]
fn on_irq() { ... }
#[allow(unused_imports)]
use hw::debug;
```

| 属性 | 付ける場所 | 意味 | 節 |
|---|---|---|---|
| `#[zp]` | `static` | 必ずゼロページに置く | [4.7](#s4-7) |
| `#[noinit]` | `static` | 起動時に初期化しない | [4.6](#s4-6) |
| `#[fixed]` | `static`、`shared` | 自動のゼロページの配置から外す | [4.7](#s4-7) |
| `#[packed]` | `struct` | 配列で 1 要素を連続して並べる | [3.8](#s3-8) |
| `#[interrupt(irq)]`、`(nmi)`、`(brk)` | `fn` | 割り込み処理にする | [10.1](#s10-1) |
| `#[inline]`、`#[inline(always)]`、`#[inline(never)]` | `fn` | 展開の指定 | [7.7](#s7-7) |
| `#[cold]` | `fn` | めったに呼ばれない | [7.7](#s7-7) |
| `#[reg(x)]`、`#[reg(y)]` | `for` | ループ変数のレジスタ | [6.4](#s6-4) |
| `#[max(n)]` | `while` | くり返しの上限 | [6.3](#s6-3) |
| `#[jump_table]` | `match` | ジャンプテーブルで実装 | [6.6](#s6-6) |
| `#[allow(名前)]`、`#[warn(名前)]`、`#[deny(名前)]` | どこでも | 警告の扱いを、その場所だけ変える | [18.2](#s18-2) |

---

