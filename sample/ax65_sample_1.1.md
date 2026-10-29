# AX65 サンプルと効果測定 1.1

『AX65 言語仕様書 1.1』（`../spec/ax65_spec_1.1.md`）の別冊です。サンプルのゲーム「METEOR DODGE」を題材に、AX65 のプロジェクトの全ファイル、最適化の段階ごとのコンパイル結果、手書きのアセンブリとの比較をまとめます。実機（6502-CPLD ボード＋HEXPAD）だけで遊べる 7セグメント版「METEOR7」も収めています。コード、コンパイル結果、測定のツールは、このフォルダ（`sample/`）に一式を固めてあります。

| 項目 | 内容 |
|---|---|
| 版 | 1.1（2026-10-29） |
| 仕様書 | AX65 言語仕様書 1.1 |
| 対象のターゲット | v2（実機：W65C02S 約 1.8425MHz、$0200 から RAM で実行、UART 19200bps、HEXPAD、割り込みなし） |
| 比べた版 | -O0、-O1、-O2（既定）、-Os、手書きのアセンブリ、cc65 版（5 つの設定。[11章](#r11)） |

## 目次

- [1. この資料について](#r1)
- [2. パッケージの構成と再現の手順](#r2)
- [3. ゲームの仕様](#r3)
- [4. サンプルのプロジェクト（全ファイル）](#r4)
- [5. ソースの解説（使っている仕様）](#r5)
- [6. コンパイル結果（想定出力）](#r6)
- [7. 手書きのアセンブリ](#r7)
- [8. 測定の方法](#r8)
- [9. 結果](#r9)
- [10. 考察](#r10)
- [11. cc65 との比較](#r11)
- [12. 7セグメント版 METEOR7](#r12)
- [13. 実機で動かす](#r13)

---

<a id="r1"></a>
## 1. この資料について

- 「コンパイル結果」は、仕様書 1.1 の規則に従って手で生成した**想定出力**です。リファレンスコンパイラ（`compiler/`）の出力ではありません。実際のコンパイラの出力とは、細部が違う可能性があります。
- 測定には、作業用に作った 65C02 のアセンブラとエミュレータを使いました（`tools/`）。エミュレータは実機 v2 の I/O（GPIO のタイマー T2、UART、HEXPAD、HEXMON の API）をまねています。サイクル数は W65C02S のデータシートの値で、ページ境界をまたぐ場合の +1、分岐の +1／+2、10進モードの `adc`・`sbc` の +1 を含みます。
- コード例と仕様の参照は、すべて仕様書 1.1 の書き方と章番号です。
- 1.0 との違い：ターゲットを実機の v2 に合わせました（割り込みを使わず、フレームの時間をタイマー T2 で計る。キーは HEXPAD、表示は UART。`$0200` に読み込み、`rts` でモニタへ戻る）。7セグメント版を加えました。

<a id="r2"></a>
## 2. パッケージの構成と再現の手順

```
sample/
├── ax65_sample_1.1.md            この資料
├── meteor/                       METEOR DODGE のプロジェクト（4章）
│   ├── ax65.proj
│   ├── targets/
│   │   ├── v2.target             実機（測定に使ったターゲット）
│   │   └── v3.target             v2 に画面を足した仮の構成
│   ├── src/
│   │   └── meteor.ax65           ゲームの本体
│   └── hw/
│       ├── acia_console.ax65     impl console（UART）
│       ├── hexpad.ax65           HEXPAD のキー
│       ├── frame_timer.ax65      タイマー T2 で 1/60 秒を計る
│       ├── hexmon_keyboard.ax65  impl keyboard（HEXMON の POLL_KEY。v2.target が bind する）
│       └── v3_screen.ax65        impl screen（v3 だけ）
├── meteor7/                      7セグメント版 METEOR7 のプロジェクト（12章）
│   ├── ax65.proj  targets/v2.target  src/meteor7.ax65
│   └── hw/  hexpad.ax65  hexmon.ax65  acia_console.ax65  hexmon_keyboard.ax65
├── stdlib/                       標準ライブラリのインターフェース（抜粋。本来は <ax65>/std/）
│   ├── console.ax65
│   └── screen.ax65
├── asm/                          コンパイル結果と手書き版（.s が原本。.bin .lst .sym.json .woz は組み立てで作る）
│   ├── meteor_O0.s  meteor_O1.s  meteor_O2.s  meteor_Os.s
│   ├── meteor_hand.s
│   └── meteor7_O2.s              METEOR7 の -O2 の想定出力（.woz が打ち込み用のダンプ）
├── cc65/                         cc65 との比較用（同じゲームの C 版。README.md を参照）
│   ├── meteor.c  crt0.s  meteor.cfg
│   ├── build.sh  build.bat       5 つの最適化の設定でビルドする
│   └── meteor_<設定>.bin .map .lbl .s   cc65 v2.18 でビルドした結果（5 つの設定）
├── tools/
│   ├── asm65.py                  65C02 のアセンブラ（2 パス、リスティングにサイクル数を出す）
│   ├── sim65c02.py               W65C02S のエミュレータ（実機 v2 の I/O と HEXMON の API）
│   ├── run_bench.py              5 つの版を組み立てて実行し、測定と出力の照合を行う
│   ├── run_meteor7.py            METEOR7 を組み立て、自動で遊ばせて確かめる
│   └── run_cc65.py               cc65 でビルドしたバイナリを、同じ条件で測定する
└── results/
    ├── bench_result.json         測定結果
    ├── cc65_result.json          cc65 版の測定結果
    ├── meteor7_result.json       METEOR7 の確認の結果
    └── meteor7_display.txt       METEOR7 の表示の変化
```

再現の手順（Python 3.8 以上。外部のパッケージは使いません）：

```
cd sample
python3 -I tools/run_bench.py
python3 -I tools/run_meteor7.py
python3 -I tools/run_cc65.py cc65     # cc65 でビルドした cc65/meteor_*.bin を測定する
```

`asm/` に `.bin`・`.lst`・`.sym.json`・`.woz` を作り、`results/` に結果を書き、要約を表示します。

AX65 のコンパイラでは、次のコマンドで同じものを作る想定です。

```
cd sample/meteor
ax65 build -t v2 -O0 -o build/O0
ax65 build -t v2 -O1 -o build/O1
ax65 build -t v2            -o build/O2      # 既定（-O2）
ax65 build -t v2 -Os -o build/Os
ax65 check --all-targets                     # v2 と v3 の両方で検査
```

<a id="r3"></a>
## 3. ゲームの仕様

`meteor/src/meteor.ax65` が実装しているゲームの仕様です。手書き版の `asm/meteor_hand.s` も、この仕様に従って作っています。

### 3.1 概要

画面の上から落ちてくる 8 個の隕石を、画面の一番下にいるプレイヤーが左右に動いてよけるゲームです。隕石が下まで落ちきるたびに 1 点が入ります。隕石に当たるか、終了のキーを押すとゲームが終わります。

### 3.2 動作環境

| 項目 | 内容 |
|---|---|
| CPU | W65C02S（65C02）、約 1.8425MHz |
| タイマー | GPIO（6522 互換、$8000）のタイマー T2 をワンショットで使う。T1 と ACR はモニタが使うので触らない |
| 入力 | HEXPAD のキー（$8800 の HP_KEY） |
| 表示 | v2：UART（6551 互換、$8400、19200bps）につないだ端末。v3（仮）：画面メモリ |
| 実行の形 | RAM の $0200 に読み込み、HEXMON の JMP キーで実行する。終わったら `rts` でモニタへ戻る |
| 割り込み | 使わない（v2 には使える割り込みがない） |

### 3.3 フィールド

- フィールドの大きさは、横 20 マス（列 0〜19）、縦 16 マス（行 0〜15）です。
- プレイヤーは常に行 15（一番下の行）にいます。
- 隕石は行 0 に現れ、行 15 まで落ち、行 16 に達した時点で消えます（画面のある構成では、行 16 に床を描きます）。

### 3.4 時間の進み方

- ゲームは 1 秒に 60 回の「フレーム」単位で進みます。
- フレームの始まりにタイマー T2 に 1/60 秒を設定して数え始め、処理が終わったら、T2 が 0 まで数えた印（IFR のビット 5）が立つのを待ちます。印は T2CL を読んで消します。約 1.8425MHz では設定値は 30706（$77F2）で、1 フレームは約 30708 サイクルです。
- 1 フレームの処理が 1/60 秒を超えた場合、次のフレームは待たずにすぐ始まります（遅れを取り戻すための追加の処理はしません）。

### 3.5 起動時の動作

1. 割り込みを止め、変数を初期化する（プレイヤーの列 10、スコア 0、乱数の種 $5A）。UART を割り込みなしの設定にする（コマンドレジスタに $0B）
2. 端末に「METEOR DODGE」と復帰（$0D）を出力する
3. 8 個の隕石を、番号 0 から 7 の順に出現させる（3.7節の「出現」）
4. タイマー T2 を数え始め、フレームのループを始める

### 3.6 1 フレームの処理

1 フレームでは、次の順序で処理します。

1. **フレームを待つ**：T2 が 0 まで数えるのを待ち、すぐに次のフレームの分を数え始める
2. **入力**：新しく押されたキーがあれば読み、操作する（3.8節）。終了のキーならゲームオーバーへ進む（この場合、このフレームの隕石の更新は行わない）
3. **隕石の更新**：8 個の隕石を、番号 0 から 7 の順に更新する（3.7節）
4. **当たりの判定**：更新の途中で 1 個でも当たりがあれば、8 個すべての更新を終えた後でゲームオーバーへ進む（この場合、このフレームの表示は行わない）
5. **表示**：フィールドの状態を表示する（3.9節）

### 3.7 隕石

#### 持っている情報

各隕石は、列（x）、行（y）、待ち時間（wait）を持ちます。

#### 出現

隕石を出現させるときは、乱数 r を 1 つ取り、次のように設定します。

| 項目 | 値 |
|---|---|
| 列 x | 出現位置の表の、r の下位 3 ビット番目の値 |
| 行 y | 0 |
| 待ち時間 wait | r の下位 2 ビット（0〜3） |

出現位置の表は、番号 i（0〜7）に対して (i × 7 + 3) mod 20 で、値は次のとおりです。

| 番号 | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|
| 列 | 3 | 10 | 17 | 4 | 11 | 18 | 5 | 12 |

#### 更新

各フレームで、各隕石を次のように更新します。

1. 待ち時間が 0 より大きければ、待ち時間を 1 減らして、この隕石の処理を終える
2. 待ち時間が 0 なら、待ち時間を 3 にし、行を 1 増やす（1 マス落ちる）
3. 落ちた結果、行が 16 になったら、その隕石を出現させ直し（新しい乱数を使う）、スコアに 1 を足す
4. そうでなく、行が 15 で、かつ列がプレイヤーの列と同じなら、当たりとする

隕石は 4 フレームに 1 マスの速さで落ちます。当たりは、隕石が行 15 に「落ちてきた」フレームにだけ判定します。

### 3.8 操作

| キー（HEXPAD） | キーコード | 動作 |
|---|---|---|
| 4 | $04 | 1 列左へ移動する。列 0 では動かない |
| 6 | $06 | 1 列右へ移動する。列 19 では動かない |
| A/D | $12 | ゲームを終了する（ゲームオーバーと同じ表示をする） |
| それ以外 | | 無視する |

- キーは 1 フレームに最大 1 つだけ読みます（HP_KEY を 1 回読む。読むと「新しいキー」の印が消える）。
- 移動は、同じフレームの隕石の更新より先に行われます。当たりの判定には、移動後の列を使います。
- RST キーは CPU のリセットなので、いつでもゲームを止めてモニタに戻れます。

### 3.9 表示

#### 画面のない構成（v2）

毎フレーム、端末に `\rSCORE dddd` を出力します。`\r` は復帰（$0D）、`dddd` はスコアの 10 進 4 桁（先頭の 0 も表示）です。同じ行に上書きされるので、端末にはスコアが 1 行で表示され続けます。

#### 画面のある構成（v3。仮）

毎フレーム、画面を消してから、床（行 16 の列 0〜19 に `-`）、隕石（各隕石の (x, y) に `*`）、プレイヤー（(プレイヤーの列, 15) に `A`）を描きます。

### 3.10 スコアと乱数

- スコアは 10 進 4 桁（BCD）で持ち、9999 点の次は 0000 点に戻ります。
- 乱数は 8 ビットのガロア型 LFSR です（種を 1 ビット左にずらし、はみ出たビットが 1 なら $1D と XOR を取る）。種の初期値は $5A です。乱数は隕石を出現させるときにだけ取るので、同じキー操作をすれば、毎回同じ展開になります。

### 3.11 ゲームオーバー

当たりがあったか、終了のキーが押されたら、`\rGAME OVER  SCORE dddd\r` を出力して、`rts` でモニタへ戻ります。

<a id="r4"></a>
## 4. サンプルのプロジェクト（全ファイル）

ファイルの置き場所は [2章](#r2) の図のとおりです。ターゲット v2 では `console` に UART の実装が bind され、`screen` は bind されません。v3 は v2 を引き継ぎ、画面を加えます。

### 4.1 `meteor/ax65.proj` — プロジェクトファイル

関連する仕様：[仕様書 17.2](../spec/ax65_spec_1.1.md#s17-2)

```
// ax65.proj — meteor プロジェクトの設定（仕様書 1.1 17.2節）
project meteor {
    main       = "src/meteor.ax65";     // プログラムの入口（main を含むファイル）
    paths      = ["."];                 // use でモジュールを探す場所（hw::acia_console → ./hw/acia_console.ax65）
    target_dir = "targets";             // ターゲット定義を探す場所
    targets    = [v2, v3];              // ax65 check --all-targets の対象
    default    = v2;                    // -t を省略したときのターゲット
    opt        = O2;                    // -O を省略したときの最適化の段階（既定も O2）
    out_dir    = "build";               // 出力先（build/<ターゲット名>/ の下に出力する）

    lints {                             // 警告の扱い（allow / warn / deny）
        naming         = warn;
        unused_imports = warn;
    }
    fmt {                               // ax65 fmt の書き方
        hex   = dollar;                 // 16進は $FF に統一する（0x にするなら hex = c）
        array = rust;                   // 配列の型は [T; N] に統一する（従来の T[N] にするなら legacy）
    }
}
```

### 4.2 `meteor/targets/v2.target` — ターゲット定義 v2（実機）

関連する仕様：[仕様書 16.7](../spec/ax65_spec_1.1.md#s16-7)

```
// targets/v2.target — 実機：6502-CPLD ボード＋HEXPAD（モニタ HEXMON）（仕様書 1.1 16.7節）
target v2 {
    cpu   = Cpu::W65C02S;               // CPU の品種
    clock = 1_842_500;                  // 約 1.8425MHz
    stack = 128;                        // プログラムが使えるスタックの予算（バイト。モニタと共用）
    run   = ram;                        // RAM に読み込んで実行する
    load  = $0200;                      // HEXMON で 0200 を表示して JMP キーで実行する
    exit  = rts;                        // rts でモニタへ戻る（起動処理はスタックポインタを設定しない）
    irq   = none;                       // 割り込みは使えない（HEXMON は IRQ ベクタを設定していない）
    nmi   = none;                       // NMI はつながっていない

    region zp    = $20..=$FF;           // $00〜$1F はモニタが使う
    region stack = $0100..=$01FF;       // モニタと共用
    region ram   = $0200..=$7EFF;       // ユーザープログラムとデータ
    region io    = [$8000..=$800F,      // GPIO（6522 互換）
                    $8400..=$8403,      // UART（6551 互換）
                    $8800..=$880F];     // HEXPAD（表示とキー）
    reserve $8004..=$8007;              // T1（モニタの時間待ちが使う）
    reserve $800B;                      // ACR（同じ）

    output = [woz, bin, sym, lst];      // woz：HEXMON に打ち込むダンプ

    bind console  = hw::acia_console;   // 1文字の入出力は UART
    bind keyboard = hw::hexmon_keyboard;    // キーは HEXMON の POLL_KEY
    const LED_DIGITS: u8 = 6;           // 7セグメント LED の桁数
    // screen は bind しない → target.has(screen) は false
}
```

### 4.3 `meteor/targets/v3.target` — ターゲット定義 v3（仮。v2 を引き継ぐ）

関連する仕様：[仕様書 16.1](../spec/ax65_spec_1.1.md#s16-1)

```
// targets/v3.target — v2 に画面を足した仮の構成（仕様書 1.1 16.7節。画面メモリの番地と形式は決まっていない）
target v3 extends v2 {
    region io = [$8000..=$800F, $8400..=$8403, $8800..=$880F,
                 $9000..=$93BF];        // 画面メモリ（40×24 文字）を I/O の区画に加える（同じ種類の区画は置き換え）
    encoding screen { ' '..='~' => $20..=$7E; }    // 画面用の文字の変換表（ASCII のまま）
    bind screen = hw::v3_screen;        // 画面の実装
    const SCREEN_WIDTH: u8 = 40;        // コードを伴わない性質は定数で書く
}
```

### 4.4 `meteor/src/meteor.ax65` — ゲームの本体

関連する仕様：[仕様書 8.1](../spec/ax65_spec_1.1.md#s8-1)

```
// src/meteor.ax65 — AX65 サンプル：隕石よけゲーム（AX65 仕様書 1.1）
//
// 想定するターゲット：
//   v2 … 実機（6502-CPLD ボード＋HEXPAD）。スコアをシリアル端末（19200bps）に表示する
//   v3 … v2 に画面を足した仮の構成。隕石とプレイヤーを画面に描く
// どちらのターゲットも割り込みがなく（irq = none）、main が終わったら rts でモニタへ戻る（exit = rts）。
// フレームの時間は GPIO のタイマー T2 で計る（hw/frame_timer.ax65）。ターゲット定義は targets/ を参照。
// console の実装は hw/acia_console.ax65、screen の実装は hw/v3_screen.ax65（v3 だけ）。
//
// 操作：HEXPAD の 4 で左、6 で右、A/D で終了

use console::putc;                      // 1文字の出力（実装はターゲットの bind で決まる）
use screen;                             // 画面（bind されていないターゲットもある）
use hw::hexpad;                         // HEXPAD のキー
use hw::frame_timer;                    // 1/60 秒のフレーム

// ---- 定数 ---------------------------------------------------------------

const FIELD_W: u8 = 20;                 // 画面の幅（マス）
const FIELD_H: u8 = 16;                 // 画面の高さ（マス）
const PLAYER_Y: u8 = FIELD_H - 1;       // プレイヤーは一番下の行にいる

const TITLE: str = p"METEOR DODGE";     // 長さ付きの文字列（ASCII）

// ---- 型 -----------------------------------------------------------------

range Mid = 0..8;                       // 隕石の番号（0〜7）。終わりの 8 は含まない

enum Key { Left, Right, Quit }          // 操作の種類

struct Meteor { x: u8, y: u8, wait: u8 }   // wait：次に1マス落ちるまでのフレーム数

// ---- 表（コンパイル時に計算する） --------------------------------------

comptime fn spawn_x(i: int) -> u8 {     // 隕石が出てくる列（コンパイル時専用の関数）
    ((i * 7 + 3) % 20) as u8            // int の計算なので、% も自由に使える
}

const SPAWN_X: [u8; Mid] = [for i in 0..8 => spawn_x(i)];   // 8個の出現位置の表

// ---- 変数 ---------------------------------------------------------------

static METEORS: [Meteor; Mid] = [Meteor { x: 0, y: 0, wait: 0 }; Mid];  // フィールドごとの配列になる
static PLAYER_X: u8 = FIELD_W / 2;  // プレイヤーの列
static SCORE: bcd16 = 0;            // スコア（10進4桁）
static SEED: u8 = $5A;              // 乱数の種（0 以外）

// ---- 乱数 ---------------------------------------------------------------

asm fn rand_step(r: u8) -> u8 {         // 8ビットの疑似乱数（ガロア型 LFSR）。r は A で届き、A で返す
    asl a                   ; A を左に1ビットずらす（はみ出たビットはキャリーへ）
    bcc done                ; はみ出たビットが 0 なら、そのまま done へ飛ぶ
    eor #$1D                ; 1 なら、決まった値と XOR を取る（周期が最大の255になる定数）
done:                       ; 合流地点
    rts                     ; A を戻り値として返す
}

fn rand() -> u8 {
    SEED = rand_step(SEED);
    SEED                                // 最後の式が戻り値になる（return を省略）
}

// ---- 入力 ---------------------------------------------------------------

fn read_key() -> Option<Key> {
    let k = hexpad::poll_key()?;        // キーが押されていなければ、この関数も None を返して抜ける
    match k {
        $04 => Some(Key::Left),         // 4 のキー
        $06 => Some(Key::Right),        // 6 のキー
        $12 => Some(Key::Quit),         // A/D のキー
        _   => None,                    // u8 は値が多いので、_ が必須
    }
}

fn move_player(k: Key) {
    match k {                           // Key の値をすべて書かないとエラーになる
        Key::Left  => { PLAYER_X = PLAYER_X.saturating_sub(1); }   // 0 より左には行かない
        Key::Right => { if PLAYER_X < FIELD_W - 1 { PLAYER_X += 1; } }
        Key::Quit  => {}
    }
}

// ---- 隕石 ---------------------------------------------------------------

fn clear_meteors() {
    #[reg(x)]                           // ループの番号を X に置く（中で関数を呼ばないので置ける）
    for m in Mid {                      // m は Mid 型なので、METEORS[m] に範囲の検査は要らない
        METEORS[m].y = 0;
        METEORS[m].wait = m as u8;      // すぐ後の respawn で上書きされる（-O2 はこの書き込みを消す。19.3節）
    }
}

fn respawn(m: Mid) {                    // 隕石を一番上の行に出し直す
    let r = rand();
    METEORS[m].x = SPAWN_X[Mid::wrap(r)];   // 下位3ビットだけを取って、必ず 0〜7 にする（and 1命令）
    METEORS[m].y = 0;
    METEORS[m].wait = r & 3;
}

fn update_meteors() -> bool {           // 隕石を動かし、プレイヤーに当たったら true を返す
    let hit = false;
    for m in Mid {
        if METEORS[m].wait > 0 {
            METEORS[m].wait -= 1;
            continue;
        }
        METEORS[m].wait = 3;
        METEORS[m].y += 1;
        if METEORS[m].y == FIELD_H {    // 下まで落ちきったら、上から出直す
            respawn(m);
            SCORE += 1;                 // BCD の足し算（sed と cld はコンパイラが入れる）
        } else if METEORS[m].y == PLAYER_Y && METEORS[m].x == PLAYER_X {
            hit = true;
        }
    }
    hit
}

// ---- 表示 ---------------------------------------------------------------

fn print(s: str) {                      // 長さ付きの文字列を出力する
    for c in s { putc(c); }
}

fn print_score() {
    for d in SCORE.digits() {           // 上の桁から1桁ずつ（0〜9）取り出す
        putc(('0' as u8 + d) as char);
    }
}

fn draw() {
    comptime if target.has(screen) {    // 画面が bind されているターゲットだけ
        screen::clear();
        for x in (0..FIELD_W).rev() {   // 逆順のループ（dex と bpl で回るので少し速い）
            screen::put(x, FIELD_H, '-'@screen);    // 床を描く（画面用の文字コードに変換済み）
        }
        for m in Mid {
            screen::put(METEORS[m].x, METEORS[m].y, '*'@screen);
        }
        screen::put(PLAYER_X, PLAYER_Y, 'A'@screen);
    } else {                            // 画面がないターゲット：スコアだけを端末に上書き表示する
        putc('\r');
        print(p"SCORE ");
        print_score();
    }
}

// ---- メイン -------------------------------------------------------------

fn main() {
    print(TITLE);
    putc('\r');

    clear_meteors();
    for m in Mid { respawn(m); }

    frame_timer::start();               // 最初のフレームの時間を計り始める
    'game: loop {
        frame_timer::wait();            // 前のフレームの始まりから 1/60 秒たつまで待つ
        frame_timer::start();           // 次のフレームの時間を計り始める
        if let Some(k) = read_key() {
            if k == Key::Quit { break 'game; }
            move_player(k);
        }
        if update_meteors() { break 'game; }
        draw();
    }

    print(p"\rGAME OVER  SCORE ");
    print_score();
    putc('\r');
}                                       // main が終わると rts でモニタへ戻る（ターゲット定義の exit = rts）
```

### 4.5 `meteor/hw/acia_console.ax65` — console の実装（UART）

関連する仕様：[仕様書 8.4](../spec/ax65_spec_1.1.md#s8-4)

```
// hw/acia_console.ax65 — console インターフェースの UART（6551 互換）による実装（仕様書 1.1 8.4節）
//   v2 の UART は 19200bps・8N1 固定。最初にコマンドレジスタへ $0B（割り込みなし）を書く。

flags AciaStatus { Pe, Fe, Ovr, Rdrf, Tdre, Dcd, Dsr, Irq }    // ビット0〜7

io(out) CMD: u8 @ $8402;                // コマンドレジスタ

init {                                  // 起動時に main の前に 1 回だけ実行される
    CMD = $0B;                          // 割り込みなしで使う
}

impl console {
    io      DATA:   u8         @ $8400; // 送信データ（書く）と受信データ（読む）
    io(in)  STATUS: AciaStatus @ $8401; // 状態（読み込み専用）

    fn putc(c: char) {                  // 送信の準備ができるまで待ってから 1 文字送る（インターフェースの関数は自動で公開される）
        while !STATUS.contains(AciaStatus::Tdre) { }
        DATA = c as u8;
    }

    fn poll() -> Option<char> {         // 受信していれば文字を返し、していなければ None
        if STATUS.contains(AciaStatus::Rdrf) { Some(DATA as char) } else { None }
    }

    fn getc() -> char {                 // 1 文字届くまで待つ
        loop {
            if let Some(c) = poll() { return c; }
        }
    }
}
```

### 4.6 `meteor/hw/hexpad.ax65` — HEXPAD のキー

関連する仕様：[仕様書 11.1](../spec/ax65_spec_1.1.md#s11-1)

```
// hw/hexpad.ax65 — HEXPAD のキー（仕様書 1.1。meteor で使う分だけ）
//   HP_KEY：ビット7 が「新しく押されたキーあり」、ビット4〜0 がキーコード。読むとビット7 が消える。

io(in) HP_KEY: u8 @ $8800;              // キーのレジスタ（読むのは poll_key の 1 か所だけにする）

pub fn poll_key() -> Option<u8> {       // 新しく押されたキーがあればキーコードを返す。待たない
    let k = HP_KEY;                     // 1 回だけ読む（2 回読むと、2 回目はビット7 が消えている）
    if k & $80 == 0 { return None; }    // 新しいキーはない
    Some(k & $1F)                       // キーコード（0〜F のキーは $00〜$0F、JMP $11、A/D $12、INC $13）
}
```

### 4.7 `meteor/hw/frame_timer.ax65` — タイマー T2 による 1/60 秒

関連する仕様：[仕様書 11.2](../spec/ax65_spec_1.1.md#s11-2)

```
// hw/frame_timer.ax65 — GPIO（6522 互換）のタイマー T2 で 1/60 秒を計る（仕様書 1.1）
//   v2 には割り込みがないので、T2 をワンショットで動かし、IFR のビット5 を見て待つ。
//   T1 と ACR はモニタの時間待ちが使っている（ターゲット定義で reserve されている）。

flags Ifr { Ca2, Ca1, Shift, Cb2, Cb1, T2, T1, Any }   // 6522 の割り込みフラグ（ビット0〜7）

io      T2C_L: u8  @ $8008;             // 書く：T2 の初期値の下位。読む：T2 のフラグが消える
io(out) T2C_H: u8  @ $8009;             // T2 の初期値の上位（書くと数え始める）
io(in)  IFR:   Ifr @ $800D;             // 割り込みフラグ（割り込みは起こさず、フラグだけを見る）

comptime assert(target.clock / 60 <= 65535, "クロックが速すぎて、1/60 秒が 16 ビットに収まらない");
const PERIOD: u16 = (target.clock / 60 - 2) as u16;   // v2 では 30706

pub fn start() {                        // 1/60 秒を数え始める
    T2C_L = (PERIOD & $FF) as u8;       // 下位を先に書く（I/O の順序はソースどおりに守られる）
    T2C_H = (PERIOD >> 8) as u8;        // 上位を書くと数え始める
}

pub fn wait() {                         // start() から 1/60 秒たつまで待つ
    while !IFR.contains(Ifr::T2) { }
    let _ = T2C_L;                      // 読み捨てて T2 のフラグを消す（bit 命令で読む）
}
```

### 4.8 `meteor/hw/hexmon_keyboard.ax65` — keyboard の実装（HEXMON の POLL_KEY）

関連する仕様：[仕様書 7.6](../spec/ax65_spec_1.1.md#s7-6)

```
// hw/hexmon_keyboard.ax65 — keyboard インターフェースの、HEXMON の POLL_KEY による実装（仕様書 1.1 8.4節）
//   v2.target が bind している。meteor は hw::hexpad を直接使うので、この実装は呼ばない。

impl keyboard {
    extern fn poll() -> Option<char> in A   // POLL_KEY：新しいキーがあればキャリー 1 で、A にキーコード
        @ $FF03 clobbers(A, flags) stack(4);
}
```

### 4.9 `meteor/hw/v3_screen.ax65` — screen の実装（v3 の画面メモリ）

関連する仕様：[仕様書 8.4](../spec/ax65_spec_1.1.md#s8-4)

```
// hw/v3_screen.ax65 — screen インターフェースの、v3（仮）の画面メモリによる実装（仕様書 1.1 8.4節）
//   v3 は仮の構成：画面メモリは $9000 から 40×24 文字、1 バイトが 1 文字（ASCII）。

range Col = 0..40;                      // 列（0〜39）
range Row = 0..24;                      // 行（0〜23）

impl screen {
    const WIDTH: u8 = 40;               // インターフェースの定数（関数と同じく自動で公開され、pub は書かない）
    const HEIGHT: u8 = 24;

    io VRAM: [[u8; Col]; Row] @ $9000;  // 画面メモリ（行ごとの先頭アドレスの表はコンパイラが作る）

    fn clear() {                        // 画面全体を空白にする
        for r in Row {
            for c in Col { VRAM[r][c] = ' '@screen as u8; }
        }
    }

    fn put(x: u8, y: u8, c: char@screen) {          // 画面の外なら何もしない
        if let Some(r) = Row::check(y) {
            if let Some(col) = Col::check(x) { VRAM[r][col] = c as u8; }
        }
    }
}
```

### 4.10 `stdlib/console.ax65` — 標準ライブラリのインターフェース console（抜粋）

関連する仕様：[仕様書 20.4](../spec/ax65_spec_1.1.md#s20-4)

```
// <ax65>/std/console.ax65 — 1文字の入出力のインターフェース（標準ライブラリ。仕様書 1.1 20.4節）
interface console {
    fn putc(c: char);                   // 1 文字送る
    fn getc() -> char;                  // 1 文字届くまで待つ
    fn poll() -> Option<char>;          // 届いていれば返す。待たない
}
```

### 4.11 `stdlib/screen.ax65` — 標準ライブラリのインターフェース screen（抜粋）

関連する仕様：[仕様書 20.4](../spec/ax65_spec_1.1.md#s20-4)

```
// <ax65>/std/screen.ax65 — 文字画面のインターフェース（標準ライブラリ。仕様書 1.1 20.4節）
interface screen {
    const WIDTH: u8;                    // 画面の幅（文字）
    const HEIGHT: u8;                   // 画面の高さ（文字）
    fn clear();                         // 画面を消す
    fn put(x: u8, y: u8, c: char@screen);   // (x, y) に 1 文字置く。画面の外なら何もしない
}
```

<a id="r5"></a>
## 5. ソースの解説（使っている仕様）

`meteor/src/meteor.ax65` と `meteor/hw/` が使っている仕様と、仕様書 1.1 の該当箇所です。

| ソースの箇所 | 使っている仕様 | 仕様書 |
|---|---|---|
| `use console::putc;`、`use screen;` | モジュールの読み込み。`console`・`screen` は標準ライブラリのインターフェースで、実装はターゲットの bind で決まる。v2 では `screen` が bind されていないが、`use` はできる | [8.2](../spec/ax65_spec_1.1.md#s8-2)、[8.4](../spec/ax65_spec_1.1.md#s8-4) |
| `use hw::hexpad;`、`use hw::frame_timer;` | 自分のマシン用のモジュール。I/O 変数はそのモジュールだけが操作し、外には関数を公開する | [8.3](../spec/ax65_spec_1.1.md#s8-3)、[11.4](../spec/ax65_spec_1.1.md#s11-4) |
| `const FIELD_W: u8 = 20;` | 定数。使った場所に値が埋め込まれ、メモリを使わない | [4.2](../spec/ax65_spec_1.1.md#s4-2) |
| `comptime assert(target.clock / 60 <= 65535, …)`（frame_timer） | コンパイル時の検査。`target.clock` はターゲット定義のクロック（v2 では 1_842_500）。説明の文字列には日本語も書ける | [13.5](../spec/ax65_spec_1.1.md#s13-5)、[2.1](../spec/ax65_spec_1.1.md#s2-1) |
| `const PERIOD: u16 = (target.clock / 60 - 2) as u16;` | コンパイル時の計算（`int` で計算し、`u16` に変換。範囲外ならエラー）。v2 では 30706 | [13.1](../spec/ax65_spec_1.1.md#s13-1)、[3.14](../spec/ax65_spec_1.1.md#s3-14) |
| `const TITLE: str = p"METEOR DODGE";` | 長さ付きの文字列 | [2.7](../spec/ax65_spec_1.1.md#s2-7)、[3.9](../spec/ax65_spec_1.1.md#s3-9) |
| `range Mid = 0..8;` | 範囲型（隕石の番号 0〜7）。`METEORS[m]` の範囲の検査が不要になる | [3.4](../spec/ax65_spec_1.1.md#s3-4) |
| `enum Key { Left, Right, Quit }` | 列挙型。`match` で網羅性が検査される | [3.5](../spec/ax65_spec_1.1.md#s3-5) |
| `flags Ifr { …, T2, T1, Any }`、`flags AciaStatus` | ビットフラグ型。`IFR.contains(Ifr::T2)` はビット 5 を調べる | [3.6](../spec/ax65_spec_1.1.md#s3-6) |
| `struct Meteor { x: u8, y: u8, wait: u8 }` | 構造体。配列はフィールドごとに並ぶ | [3.7](../spec/ax65_spec_1.1.md#s3-7)、[3.8](../spec/ax65_spec_1.1.md#s3-8) |
| `comptime fn spawn_x(i: int) -> u8` と `[for i in 0..8 => spawn_x(i)]` | コンパイル時専用の関数と、表の生成。結果は 3, 10, 17, 4, 11, 18, 5, 12 | [7.3](../spec/ax65_spec_1.1.md#s7-3)、[13.3](../spec/ax65_spec_1.1.md#s13-3) |
| `static SCORE: bcd16 = 0;`、`SCORE += 1;`、`SCORE.digits()` | BCD の型、足し算（`sed`・`cld` はコンパイラが入れる）、桁の取り出し | [5.5](../spec/ax65_spec_1.1.md#s5-5) |
| `io(in) HP_KEY`、`io(out) T2C_L` など | I/O 変数。読み書きの回数と順序が守られる（HP_KEY は 1 回だけ読む。T2 は下位を先に書く） | [11.1](../spec/ax65_spec_1.1.md#s11-1)、[11.2](../spec/ax65_spec_1.1.md#s11-2) |
| `init { CMD = $0B; }`（acia_console） | 起動時に 1 回だけ実行される | [8.5](../spec/ax65_spec_1.1.md#s8-5) |
| `extern fn poll() -> Option<char> in A @ $FF03 …`（hexmon_keyboard） | ROM ルーチンの宣言。有無はキャリー、値は A | [7.6](../spec/ax65_spec_1.1.md#s7-6) |
| `asm fn rand_step(r: u8) -> u8` | 関数全体のアセンブリ（A で受け取り、A で返す） | [7.4](../spec/ax65_spec_1.1.md#s7-4)、[14章](../spec/ax65_spec_1.1.md#s14) |
| `fn read_key() -> Option<Key>`、`hexpad::poll_key()?` | Option と `?`。戻り値の有無はキャリーで返る | [3.11](../spec/ax65_spec_1.1.md#s3-11)、[15.3](../spec/ax65_spec_1.1.md#s15-3) |
| `match k { $04 => …, _ => None }` | `match`。`u8` は値が多いので `_` が必須 | [6.6](../spec/ax65_spec_1.1.md#s6-6) |
| `PLAYER_X.saturating_sub(1)` | 0 で止まる引き算 | [5.2](../spec/ax65_spec_1.1.md#s5-2) |
| `#[reg(x)] for m in Mid` | ループ変数のレジスタの指定 | [6.4](../spec/ax65_spec_1.1.md#s6-4) |
| `Mid::wrap(r)` | 下位ビットだけを取って範囲型にする（`and #7`） | [3.4](../spec/ax65_spec_1.1.md#s3-4) |
| `continue`、`'game: loop`、`break 'game` | ループの制御とラベル | [6.5](../spec/ax65_spec_1.1.md#s6-5) |
| `for c in s { putc(c); }` | 長さ付きの文字列を回す。`str` の引数はポインタで、A と X で渡る | [3.9](../spec/ax65_spec_1.1.md#s3-9)、[15.2](../spec/ax65_spec_1.1.md#s15-2) |
| `comptime if target.has(screen) { … } else { … }` | bind の有無による条件コンパイル。v2 では else 側だけが残る | [13.6](../spec/ax65_spec_1.1.md#s13-6)、[16.6](../spec/ax65_spec_1.1.md#s16-6) |
| `'-'@screen` | 画面の変換表で変換した文字（v3 の `encoding screen`） | [3.3](../spec/ax65_spec_1.1.md#s3-3)、[16.5](../spec/ax65_spec_1.1.md#s16-5) |
| ターゲット定義の `exit = rts`、`irq = none`、`reserve $8004..=$8007;` | rts でモニタへ戻る。割り込みは使えない。モニタが使う T1 と ACR には I/O 変数を宣言できない | [16.3](../spec/ax65_spec_1.1.md#s16-3)、[16.4](../spec/ax65_spec_1.1.md#s16-4) |

コンパイラがこのプログラムについて確かめること：

| 検査 | このプログラムでの内容 |
|---|---|
| 再帰がない | すべての呼び出し関係に輪がない |
| スタックの予算 | 最も深い経路は main → update_meteors → respawn → rand → rand_step（-O0 で 10 バイト。モニタへの戻り番地を含む。予算 128 バイト） |
| 範囲外の添字がない | `METEORS` と `SPAWN_X` の添字は、すべて `Mid` 型 |
| 値を入れる前の読み込みがない | すべてのローカル変数は、宣言と同時に値が入っている |
| 割り込みの機能を使っていない | v2 は `irq = none`。`#[interrupt]`、`irq_enable()`、`shared` を使えば、エラーになる |
| モニタの資源 | ゼロページは $20 から。T1・ACR（reserve）には I/O 変数を宣言していない |
| I/O の向き | `io(in)` の `HP_KEY`・`IFR` には書かず、`io(out)` の `T2C_L`・`T2C_H`・`CMD` は読んでいない |
| match の網羅性 | `move_player` の `match` は `Key` のすべての値を書いている |
| Option の確認 | `read_key()`・`poll_key()` の結果は、`if let` と `?` で確かめてから使っている |
| bind の確認 | `screen` は `comptime if target.has(screen)` の中だけで使っている |

<a id="r6"></a>
## 6. コンパイル結果（想定出力）

どの段階にも、段階にかかわらず行う規則（[仕様書 19.5](../spec/ax65_spec_1.1.md#s19-5)）が入っています。初期値 0 は `stz`、末尾呼び出しは `jmp`、ポインタの引数は A と X です。v2 は `exit = rts`・`irq = none` なので、起動処理はスタックポインタを設定せず、割り込みベクタも書きません。

| ファイル | 段階 | 主な最適化 |
|---|---|---|
| `asm/meteor_O0.s` | -O0 | なし。変数は通常の RAM、ゼロページはポインタだけ。関数は展開しない |
| `asm/meteor_O1.s` | -O1 | ゼロページへの自動配置。1 か所からしか呼ばれない関数と、小さな `respawn`・`frame_timer::start` を展開。`clear_meteors` のループを逆順にした |
| `asm/meteor_O2.s` | -O2（既定） | ゼロページへの自動配置。展開は 1 か所からしか呼ばれない関数だけ（`respawn` と `frame_timer::start` は関数のまま。`respawn` の番号は X で渡す）。`clear_meteors` の不要な書き込みを削除。`digits()` を結果領域なしで展開し、共通部分を `print_bcd` にくくり出し。`print` のループがページ境界をまたがないように `ft_start` を最後に置いた |
| `asm/meteor_Os.s` | -Os | -O2 に加えて `respawn`、`frame_timer::start` を展開。`digits()` の展開の共通部分はくくり出さない |

段階による違いが最も大きいのは、隕石の更新のループです。

```asm
; -O0：respawn が X と Y を壊すので、隕石の番号 m をメモリ（um_m）に置く
@loop:  ldx um_m                ; m を X に読む（毎回）
        lda M_WAIT,x            ; METEORS[m].wait を読む（通常の RAM なので 4 サイクル、3 バイト）
        ...                     ; （中略）
@next:  inc um_m                ; m を 1 増やす（メモリ上で）
        lda um_m                ; m を読む
        cmp #8                  ; 8 に達したか
        bne @loop               ; 1 回あたりの管理のコストは 19 サイクル

; -O2：respawn に m を X で渡し、respawn は X を変えないので、m を X に置いたまま回せる
upd_loop:                       ; ループの先頭
        lda M_WAIT,x            ; METEORS[m].wait（ゼロページなので 2 バイト）
        ...                     ; （中略）
upd_next:                       ; 次の隕石へ
        inx                     ; 次の m へ
        cpx #8                  ; 8 に達したか
        bne upd_loop            ; 1 回あたりの管理のコストは 7 サイクル
```

フレームを待つ部分は、-O2 では `bit` 命令を使います。

```asm
bench_loop:                     ; 'game: loop の先頭（frame_timer::wait を展開）
        lda #$20                ; IFR.contains(Ifr::T2)：T2 のビット（ビット5）を A に
@wait:  bit IFR                 ; A と IFR の AND で Z フラグが決まる（A は変わらない）
        beq @wait               ; T2 のビットが 0 なら待つ
        bit T2C_L               ; let _ = T2C_L：読み捨てて T2 のフラグを消す（A は変わらない）
```

<a id="r7"></a>
## 7. 手書きのアセンブリ

`asm/meteor_hand.s` は、[3章](#r3) のゲームの仕様から、最初からアセンブリで書いたものです。端末への出力が、コンパイル版とまったく同じになるように書いています。

手書きならではの工夫は次のとおりです。

- **変数の置き場所**：よく使う変数と隕石の配列を、すべてゼロページに置く
- **呼び出しの展開**：乱数、出し直し、入力の判定、フレームの時間の設定（ループの中だけ）を、呼び出さずにその場に書く
- **待ち方**：T2 のフラグも、UART の送信の準備も、`bit` 命令で待つ（文字を A に入れたまま待てる）。T2 のフラグは `bit` で T2CL を読み捨てて消す
- **プログラムの書き方**：文字列を 0 終端で 1 つの表にまとめ、表の中の位置（X）で渡す。スコアの繰り上がりがなければ上位の足し算を飛ばす

最後の項目はゲームの仕様を変えない範囲の書き方の違いで、AX65 のソースでも同じように書けます。

<a id="r8"></a>
## 8. 測定の方法

| 項目 | 内容 |
|---|---|
| 実行 | `tools/run_bench.py` が 5 つの版を `tools/asm65.py` で組み立て、`tools/sim65c02.py` で実行する |
| ハードウェアのモデル | 実機 v2：W65C02S（約 1.8425MHz）、GPIO のタイマー T2（ワンショット）、UART（送信）、HEXPAD のキー。プログラムはモニタから `jsr` で呼ばれた状態で始め、最後の `rts` で終わる。割り込みの命令（`cli`、`wai`）、ゼロページ $00〜$1F への書き込み、T1・ACR の読み書きがあれば、エラーで止める |
| 入力 | 5 つの版に、同じキー操作を同じフレームで与える（4 と 6 のキーでの移動、使わないキー、600 フレーム目に A/D） |
| 条件 1：理想 | UART は常に送信可能（CPU の処理だけを比べる） |
| 条件 2：シリアル | UART は 1 文字ごとに 960 サイクル送信中になる（約 1.8425MHz で 19200bps）。送信待ちの時間も含めて測る |
| 条件 3：衝突 | 隕石に当たって終わる別のキー操作（57 フレームで終わる） |
| 測る範囲 | 計測用のラベル `bench_frame_begin`・`bench_upd`・`bench_draw`・`bench_frame_end` の間のサイクル数（フレームの始まりの T2 の設定は「入力」に含む）。T2 を待つ間は、エミュレータが時間を一気に進める（測る範囲の外） |
| 動作の確認 | 端末への出力が、すべての版・すべての条件で一致すること |

<a id="r9"></a>
## 9. 結果

5 つの版の端末への出力は、理想・シリアル・衝突のすべての条件で一致しました（理想とシリアル：6625 バイト、最後の行「GAME OVER  SCORE 0072」。衝突：全版 57 フレームで終了）。

### 9.1 大きさ（バイト）

| 項目 | -O0 | -O1 | -O2（既定） | -Os | 手書き |
|---|---|---|---|---|---|
| コード（機械語） | 493 | 368 | 312 | 352 | 282 |
| 定数（表、文字列） | 47 | 47 | 47 | 47 | 47 |
| 読み込むファイル全体 | 540 | 415 | 359 | 399 | 329 |
| HEXMON で打ち込むキーの数（1 バイト 3 回） | 1620 | 1245 | 1077 | 1197 | 987 |
| 手書きとの比（コード） | 1.75 | 1.30 | 1.11 | 1.25 | 1.00 |

### 9.2 速さ：理想の条件（サイクル）

| 項目 | -O0 | -O1 | -O2（既定） | -Os | 手書き |
|---|---|---|---|---|---|
| **1 フレーム（平均）** | **1058.0** | **807.5** | **761.9** | **737.5** | **734.3** |
| 1 フレーム（最小〜最大） | 1004〜1521 | 762〜1075 | 715〜1076 | 692〜1005 | 688〜1029 |
| 　入力（平均。T2 の設定を含む） | 68.7 | 23.4 | 31.3 | 19.3 | 19.3 |
| 　隕石の更新（平均） | 375.3 | 231.1 | 232.6 | 231.1 | 231.0 |
| 　表示（平均） | 614 | 553 | 498 | 487 | 484 |
| 起動（タイトル表示まで含む） | 1890 | 1317 | 1304 | 1196 | 1085 |
| スタックの使用量（バイト。モニタへの戻り番地を含む） | 10 | 6 | 9 | 6 | 10 |
| 手書きとの比（1 フレーム） | 1.441 | 1.100 | 1.038 | 1.004 | 1.000 |
| -O0 からの短縮 | 0.0 % | 23.7 % | 28.0 % | 30.3 % | 30.6 % |
| CPU の使用率（1 フレーム 30708 サイクルに対して） | 3.4 % | 2.6 % | 2.5 % | 2.4 % | 2.4 % |

### 9.3 速さ：シリアル（19200bps）の条件（サイクル）

| 項目 | -O0 | -O1 | -O2（既定） | -Os | 手書き |
|---|---|---|---|---|---|
| **1 フレーム（平均。送信待ちを含む）** | **10265.0** | **10050.5** | **10058.9** | **10043.5** | **10016.3** |
| 1 フレーム（最大） | 10728 | 10318 | 10373 | 10311 | 10311 |
| 手書きとの比 | 1.025 | 1.003 | 1.004 | 1.003 | 1.000 |
| 1 フレーム（30708）を超えたフレーム | 0 | 0 | 0 | 0 | 0 |
| 起動（タイトル表示まで含む） | 13104 | 12549 | 12536 | 12428 | 12250 |

生のデータは `results/bench_result.json` にあります。

<a id="r10"></a>
## 10. 考察

### 10.1 既定の -O2 は、最も小さく、手書きとの差は 4 %

-O2 の 1 フレームは 761.9 サイクルで、手書き（734.3 サイクル）との差は 3.8 % です。-O0 からは 28 % 短くなりました。コードは 312 バイトで手書きの 1.11 倍、4 つの段階の中で最も小さくなっています。HEXMON で打ち込むキーの数は、-O0 の 1620 回に対して 1077 回です。手で打ち込む v2 では、大きさを優先する既定の -O2 が向いています。

差が最も大きく縮んだのは隕石の更新（375.3 → 232.6 サイクル）です。[6章](#r6) のとおり、-O0 では `respawn` が X を壊すためにループの番号をメモリに置くしかなく、-O2 は番号を X で渡すように渡し方を変えたので、番号を X に置いたまま回せます。

### 10.2 -O2 と -Os の差は、2 か所から呼ばれる小さな関数

-O2 の入力の処理（31.3 サイクル）が -Os・手書き（19.3 サイクル）より 12 サイクル長いのは、2 か所から呼ばれる `frame_timer::start` を関数のまま `jsr` で呼んでいるためです（`jsr` と `rts` で 12 サイクル）。-Os はこれと `respawn`、`digits()` を展開し、737.5 サイクル（手書きとの差 0.4 %）になりましたが、コードは 40 バイト大きくなりました。

### 10.3 -O1 は -O2 より遅く、大きい

-O1 は小さな関数を展開するので、隕石の更新は -Os と同じ速さ（231.1 サイクル）です。一方、表示の `digits()` を結果領域に作ってから回すため（-O2 と -Os は結果領域なしで展開する）、表示が遅く（553 対 498 サイクル）、コードも -O2 より 56 バイト大きくなりました。

### 10.4 実際のシリアル速度では、差はほとんど見えない

19200bps のシリアルでは、1 フレームの表示の 11 文字だけで約 10000 サイクルの送信待ちが生じます。そのため、1 フレームの時間は -O0 の 10265.0 サイクルから手書きの 10016.3 サイクルまで、差は 2.5 % にとどまりました。どの版も 1 フレーム（30708 サイクル）の 3 分の 1 ほどで、ゲームとしての動作は同じです。

### 10.5 まとめ

| 段階 | 1 フレーム（理想） | 手書きとの比 | 1 フレーム（シリアル） | コード | 打ち込むキーの数 |
|---|---|---|---|---|---|
| -O0 | 1058.0 | 1.441 | 10265.0 | 493 | 1620 |
| -O1 | 807.5 | 1.100 | 10050.5 | 368 | 1245 |
| **-O2（既定）** | **761.9** | **1.038** | **10058.9** | **312** | **1077** |
| -Os | 737.5 | 1.004 | 10043.5 | 352 | 1197 |
| 手書き | 734.3 | 1.000 | 10016.3 | 282 | 987 |

<a id="r11"></a>
## 11. cc65 との比較

`cc65/` の C 版（`meteor.c`、`crt0.s`、`meteor.cfg`）を、1.1 のゲームの仕様（実機 v2、タイマー T2、HEXPAD のキー、UART、`rts` で戻る）に合わせて書き直し、cc65 v2.18（Ubuntu 2.18-1）の 5 つの設定でビルドしたものを、AX65 版と同じエミュレータ・同じキー操作・同じ 3 つの条件で測定しました（`tools/run_cc65.py`）。

| 項目 | 内容 |
|---|---|
| ソース | `cc65/meteor.c`（同じゲームの C 版）、`cc65/crt0.s`（起動処理。モニタのスタックをそのまま使い、`rts` で戻る）、`cc65/meteor.cfg`（$0200 から、ゼロページは $20〜、C のスタックの先頭は $7EF0） |
| 最適化の設定 | `O0`（なし）、`O`（`-O`）、`Oirs`（`-Oirs`）、`Oirs_Cl`（`-Oirs -Cl`）、`Oirs_Cl_cs`（`-Oirs -Cl --codesize 500`） |
| 計測の目印 | `$7F00`〜`$7F04` への `stz`（4 サイクル。測定の側で差し引く）。実機で遊ぶときは `-DNO_MARK` を付けて作る |

5 つの設定のすべてで、端末への出力は AX65 の `-O2` 版と、理想・シリアル・衝突の 3 つの条件で一致しました（衝突：全設定 57 フレームで終了）。生のデータは `results/cc65_result.json` にあります。

### 11.1 結果

「コード」は STARTUP と CODE のセグメントの合計です（AX65 の「コード」と同じく、定数を含まない）。比は AX65 の既定の `-O2`（1 フレーム 761.9 サイクル、コード 312 バイト、ファイル 359 バイト、シリアル 10058.9 サイクル）に対する値です。

| 項目 | O0 | O | Oirs | Oirs_Cl | Oirs_Cl_cs | AX65 -O2 |
|---|---|---|---|---|---|---|
| **1 フレーム（理想、平均）** | **5097.8** | **2842.0** | **2545.9** | **2304.5** | **2148.5** | **761.9** |
| 1 フレーム（最小〜最大） | 4395〜8396 | 2698〜4107 | 2406〜3807 | 2205〜3242 | 2049〜3086 | 715〜1076 |
| 　入力（平均） | 188.1 | 108.3 | 108.3 | 65.3 | 65.3 | 31.3 |
| 　隕石の更新（平均） | 2210.7 | 868.7 | 872.6 | 674.2 | 674.2 | 232.6 |
| 　表示（平均） | 2699 | 1865 | 1565 | 1565 | 1409 | 498 |
| 起動（タイトル表示まで含む） | 13305 | 6134 | 5734 | 4987 | 4675 | 1304 |
| コード（バイト） | 1667 | 922 | 892 | 830 | 821 | 312 |
| 読み込むファイル全体（バイト） | 1710 | 965 | 935 | 873 | 864 | 359 |
| 1 フレーム（シリアル、平均） | 12793.8 | 10835.9 | 10818.2 | 10706.4 | 10712.4 | 10058.9 |
| AX65 -O2 との比（1 フレーム、理想） | 6.69 | 3.73 | 3.34 | 3.02 | **2.82** | 1.00 |
| AX65 -O2 との比（コード） | 5.34 | 2.96 | 2.86 | 2.66 | **2.63** | 1.00 |
| AX65 -O2 との比（シリアル） | 1.272 | 1.077 | 1.075 | 1.064 | 1.065 | 1.000 |

ハードウェアのスタックの使用量は、どの設定も 12 バイトです（このほかに、C のソフトウェアスタックを $7EF0 から下に使う）。

### 11.2 考察

cc65 の最速の設定（`-Oirs -Cl --codesize 500`）でも、1 フレームの時間は AX65 の `-O2` の 2.82 倍、コードは 2.63 倍でした。1.0 の構成での比較（2.85 倍、2.4 倍）とほぼ同じです。差の大きい部分は次のとおりです。

| 区間 | cc65（Oirs_Cl_cs） | AX65 -O2 | おもな理由（cc65 の出力 `meteor_Oirs_Cl_cs.s` から） |
|---|---|---|---|
| 隕石の更新 | 674.2 | 232.6 | ループの番号をメモリ（BSS）に置き、毎回 `lda`/`ldy` で読み直す。`m_wait[i]--` や `m_y[i]++` を、配列の番地と番号を足して `ptr1` に作り、`(ptr1)` で読み書きする（AX65 は `dec m_wait,x`） |
| 表示 | 1409 | 498 | 1 文字を送る `putc_acia(c)` が、毎回引数を `pusha` で C のスタックに積み、`(sp)` から読む。`print` は文字列のポインタを `pushax` で C のスタックに置き、1 文字ごとに `(sp)` から `ptr1` に読み直して 16 ビットで足す（AX65 は Y を添字にして `lda (p),y`） |
| 入力 | 65.3 | 31.3 | `move_player(key)` が引数を `pusha` で C のスタックに積み、`(sp)` から読む（AX65 は A で受け取る） |

シリアル（19200bps）の条件では送信待ちが大部分を占めるので、差は 6.5 % にとどまります。ただし、打ち込むキーの数（1 バイト 3 回）は、AX65 の `-O2` の 1077 回に対して、cc65 の最小の設定でも 2592 回です。

<a id="r12"></a>
## 12. 7セグメント版 METEOR7

`meteor7/` は、実機の HEXPAD（7セグメント LED 6 桁とキー）だけで遊べる、小さな隕石よけです。端末はいりません。

### 12.1 遊び方

- 6 桁がそれぞれ 1 本の列です。上に隕石がある列は「-」、自分がいる列は「u」、両方なら「o」で表示します。
- 隕石は少しすると落ちます。そのとき自分がその列にいたら終わりです。よけるたびに 1 点が入り、隕石は少しずつ速くなります。
- 4 のキーで左、6 のキーで右へ動きます。A/D で終了します。
- 終わると、左の 2 桁に「--」、右の 4 桁に点数（10 進）が出ます。何かキーを押すとモニタへ戻ります。

### 12.2 ファイル


#### meteor7/src/meteor7.ax65 — ゲームの本体

```
// src/meteor7.ax65 — AX65 サンプル：7セグメント版の隕石よけ（AX65 仕様書 1.1）
//
// ターゲット：v2（実機。HEXPAD の 7セグメント LED 6 桁とキーだけで遊べる）
// 遊び方：
//   ・6 桁がそれぞれ 1 本の列。上に隕石があると「-」、自分がいる列は「u」、両方なら「o」
//   ・隕石は少しすると落ちる。そのとき自分がその列にいたら終わり。よけるたびに 1 点
//   ・4 のキーで左、6 のキーで右、A/D で終了。終わると右の 4 桁に点数が出る。何かキーを押すとモニタへ戻る

use hw::hexpad;                         // 表示とキー
use hw::hexmon::delay_ms;               // HEXMON の時間待ち

const MIN_SPEED: u8 = 20;               // いちばん速いとき：1 段を 20 × 10ms で進める

static METEOR: [bool; hexpad::Digit] = [false; hexpad::Digit];   // その列の上に隕石があるか
static PLAYER: hexpad::Digit = 2;       // 自分の列
static SCORE: bcd16 = 0;                // 点数（10進4桁）
static SEED: u8 = $5A;                  // 乱数の種（0 以外）
static SPEED: u8 = 60;                  // 1 段の間にキーを調べる回数（10ms ごと）。よけるたびに減る

asm fn rand_step(r: u8) -> u8 {         // 8ビットの疑似乱数（ガロア型 LFSR）。r は A で届き、A で返す
    asl a                   ; A を左に1ビットずらす（はみ出たビットはキャリーへ）
    bcc done                ; はみ出たビットが 0 なら、そのまま done へ飛ぶ
    eor #$1D                ; 1 なら、決まった値と XOR を取る
done:                       ; 合流地点
    rts                     ; A を戻り値として返す
}

fn rand() -> u8 {
    SEED = rand_step(SEED);
    SEED
}

fn draw() {                             // 6 桁を描き直す
    for l in hexpad::Digit {
        let code = if METEOR[l] {
            if l == PLAYER { hexpad::SMALL_O } else { hexpad::MINUS }
        } else if l == PLAYER {
            hexpad::SMALL_U
        } else {
            hexpad::BLANK
        };
        hexpad::show(l, code);
    }
}

fn move_player(k: u8) {
    match k {
        $04 => {                        // 4 のキー：左（桁の番号が大きい方）へ
            if let Some(p) = hexpad::Digit::check(PLAYER as u8 + 1) { PLAYER = p; }
        }
        $06 => {                        // 6 のキー：右へ（0 から引くと 255 になり、check で None になる）
            if let Some(p) = hexpad::Digit::check(PLAYER as u8 - 1) { PLAYER = p; }
        }
        _ => {}
    }
}

fn step() -> bool {                     // 隕石を 1 段進める。当たったら true
    for l in hexpad::Digit {            // 上にある隕石が落ちる
        if METEOR[l] {
            if l == PLAYER { return true; }
            METEOR[l] = false;
            SCORE += 1;                 // BCD の足し算
            if SPEED > MIN_SPEED { SPEED -= 1; }
        }
    }
    let r = rand();                     // 新しい隕石を出す
    if let Some(l) = hexpad::Digit::check(r & 7) { METEOR[l] = true; }      // 6 と 7 なら出さない
    if r & $C0 == 0 {                   // 4 回に 1 回は、もう 1 個（列はビット3〜5 で決める）
        if let Some(l) = hexpad::Digit::check((r >> 3) & 7) { METEOR[l] = true; }
    }
    false
}

fn main() {
    'game: loop {
        draw();
        for _ in 0..SPEED {             // SPEED 回、10ms ごとにキーを調べる
            delay_ms(10);
            if let Some(k) = hexpad::poll_key() {
                if k == $12 { break 'game; }    // A/D：終了
                move_player(k);
                draw();
            }
        }
        if step() { break 'game; }
    }
    delay_ms(250);                      // ぶつかったところを少し見せる
    delay_ms(250);
    hexpad::show(5, hexpad::MINUS);     // 左の 2 桁は「--」
    hexpad::show(4, hexpad::MINUS);
    hexpad::show_bcd(SCORE);            // 右の 4 桁に点数
    loop {                              // 何かキーが押されるまで待つ
        if let Some(_) = hexpad::poll_key() { break; }
    }
}                                       // main が終わると rts でモニタへ戻る（exit = rts）
```

#### meteor7/hw/hexpad.ax65 — HEXPAD の表示とキー

```
// hw/hexpad.ax65 — HEXPAD（7セグメント LED 6 桁とキー 20 個）（仕様書 1.1。meteor7 で使う分だけ）
//   HP_KEY：ビット7 が「新しく押されたキーあり」、ビット4〜0 がキーコード。読むとビット7 が消える。
//   HP_DIGIT0〜5：1 桁ずつ表示コード（0〜31）を書く（HP_DIGIT0 が右端、HP_DIGIT5 が左端）。

pub range Digit = 0..6;                 // 桁の位置（0 = 右端 … 5 = 左端）

io(in)  HP_KEY:   u8          @ $8800;  // キーのレジスタ（読むのは poll_key の 1 か所だけにする）
io(out) HP_DIGIT: [u8; Digit] @ $8808;  // 1 桁ずつの表示（$8808〜$880D）

pub const MINUS:  u8 = 16;              // 表示コード：-（真ん中の横線）
pub const BLANK:  u8 = 20;              // 表示コード：空白
pub const SMALL_O: u8 = 0;              // 表示コード：o の形（下半分の四角）
pub const SMALL_U: u8 = 31;             // 表示コード：u の形

pub fn poll_key() -> Option<u8> {       // 新しく押されたキーがあればキーコードを返す。待たない
    let k = HP_KEY;                     // 1 回だけ読む（2 回読むと、2 回目はビット7 が消えている）
    if k & $80 == 0 { return None; }    // 新しいキーはない
    Some(k & $1F)                       // キーコード（0〜F のキーは $00〜$0F、JMP $11、A/D $12、INC $13）
}

#[inline]
pub fn show(d: Digit, code: u8) {       // 1 桁に表示コードを書く（1 命令なので、呼んだ場所に展開する）
    HP_DIGIT[d] = code;
}

pub fn show_bcd(v: bcd16) {             // 右の 4 桁に、10進 4 桁を表示する（表示コード 0〜9 は数字そのもの）
    let p: u8 = 4;
    for d in v.digits() {               // 上の桁から 1 桁ずつ（0〜9）
        p -= 1;
        if let Some(pos) = Digit::check(p) { HP_DIGIT[pos] = d; }
    }
}
```

#### meteor7/hw/hexmon.ax65 — HEXMON の API（extern fn）

```
// hw/hexmon.ax65 — HEXMON の API（仕様書 1.1 7.6節。meteor7 で使う分だけ）
//   使うスタックはガイドに書かれていないので、多めに見積もっている。

pub extern fn delay_ms(ms: u8 in A)     // DELAY_MS：A ミリ秒待つ（0 は 256）
    @ $FF09 clobbers(A, flags) stack(4);
```

#### meteor7/ax65.proj — プロジェクトファイル

```
// ax65.proj — meteor7（7セグメント版）のプロジェクトの設定（仕様書 1.1 17.2節）
project meteor7 {
    main       = "src/meteor7.ax65";    // main を含むファイル
    paths      = ["."];                 // use でモジュールを探す場所
    target_dir = "targets";             // ターゲット定義を探す場所
    targets    = [v2];                  // 実機だけ
    default    = v2;                    // -t を省略したときのターゲット
    opt        = O2;                    // 大きさ優先（手で打ち込むので小さい方がよい）
    out_dir    = "build";               // 出力先
}
```

`meteor7/targets/v2.target`、`meteor7/hw/acia_console.ax65`、`meteor7/hw/hexmon_keyboard.ax65` は、`meteor/` と同じものです（v2.target が bind するので置いてある）。

### 12.3 使っている仕様

| ソースの箇所 | 使っている仕様 | 仕様書 |
|---|---|---|
| `pub extern fn delay_ms(ms: u8 in A) @ $FF09 clobbers(A, flags) stack(4);` | HEXMON の API の宣言。A とフラグだけを壊すので、X と Y は呼び出しをまたいで使える | [7.6](../spec/ax65_spec_1.1.md#s7-6) |
| `pub range Digit = 0..6;`、`io(out) HP_DIGIT: [u8; Digit] @ $8808;` | 範囲型を添字にする I/O の配列（6 桁の表示） | [3.4](../spec/ax65_spec_1.1.md#s3-4)、[11.1](../spec/ax65_spec_1.1.md#s11-1) |
| `#[inline] pub fn show(d: Digit, code: u8)` | 展開の指定（3 か所から呼ばれるが、1 命令なので -O2 でも展開する） | [7.7](../spec/ax65_spec_1.1.md#s7-7)、[19.3](../spec/ax65_spec_1.1.md#s19-3) |
| `static METEOR: [bool; hexpad::Digit]` | 他のモジュールの範囲型を添字にした `bool` の配列 | [3.2](../spec/ax65_spec_1.1.md#s3-2)、[3.8](../spec/ax65_spec_1.1.md#s3-8) |
| `hexpad::Digit::check(PLAYER as u8 + 1)` | 範囲の検査と `Option`（0 から引くと 255 になり、`None` になる） | [3.4](../spec/ax65_spec_1.1.md#s3-4)、[5.2](../spec/ax65_spec_1.1.md#s5-2) |
| `for _ in 0..SPEED` | 変数の範囲の `for` | [6.4](../spec/ax65_spec_1.1.md#s6-4) |
| `for l in hexpad::Digit { … return true; … }` | 途中で `return` するループ（逆順にはしない） | [6.4](../spec/ax65_spec_1.1.md#s6-4)、[6.7](../spec/ax65_spec_1.1.md#s6-7) |
| `v.digits()` と `HP_DIGIT[pos] = d;` | BCD の桁をそのまま表示コードにする（表示コード 0〜9 は数字） | [5.5](../spec/ax65_spec_1.1.md#s5-5) |

### 12.4 想定出力と打ち込み用のダンプ

`asm/meteor7_O2.s` が -O2 の想定出力で、268 バイトです（HEXMON で打ち込むキーは 804 回）。`tools/run_meteor7.py` で、簡単なプログラムに遊ばせて動きを確かめました。自分の列に隕石が来てから 0.25 秒後に空いている列へ動くようにすると、40 段目で間に合わなくなって終わり、点数は 41 点、表示は「--0041」、キーを押すとモニタへ戻りました（16.7 秒、スタックの使用量は 4 バイト）。表示の変化は `results/meteor7_display.txt` にあります。

打ち込み用のダンプ（`asm/meteor7_O2.woz`）：


```
0200: 78 D8 A9 02 85 26 A9 5A
0208: 85 29 A9 3C 85 2A 64 27
0210: 64 28 A2 05 74 20 CA 10
0218: FB 20 DC 02 A5 2A F0 2D
0220: 85 2B A9 0A 20 09 FF 20
0228: 01 03 90 1D C9 12 F0 74
0230: C9 04 D0 05 A6 26 E8 80
0238: 07 C9 06 D0 09 A6 26 CA
0240: E0 06 B0 02 86 26 20 DC
0248: 02 C6 2B D0 D5 A2 00 B5
0250: 20 F0 1D E4 26 F0 4D 74
0258: 20 F8 18 A5 27 69 01 85
0260: 27 A5 28 69 00 85 28 D8
0268: A5 2A C9 15 90 02 C6 2A
0270: E8 E0 06 D0 DA A5 29 0A
0278: 90 02 49 1D 85 29 85 2C
0280: 29 07 C9 06 B0 05 AA A9
0288: 01 95 20 A5 2C 29 C0 D0
0290: 10 A5 2C 4A 4A 4A 29 07
0298: C9 06 B0 05 AA A9 01 95
02A0: 20 4C 19 02 A9 FA 20 09
02A8: FF A9 FA 20 09 FF A9 10
02B0: 8D 0D 88 8D 0C 88 A5 28
02B8: 4A 4A 4A 4A 8D 0B 88 A5
02C0: 28 29 0F 8D 0A 88 A5 27
02C8: 4A 4A 4A 4A 8D 09 88 A5
02D0: 27 29 0F 8D 08 88 20 01
02D8: 03 90 FB 60 A2 00 B5 20
02E0: F0 0C E4 26 F0 04 A9 10
02E8: 80 0E A9 00 80 0A E4 26
02F0: D0 04 A9 1F 80 02 A9 14
02F8: 9D 08 88 E8 E0 06 D0 DE
0300: 60 AD 00 88 10 04 29 1F
0308: 38 60 18 60
```

<a id="r13"></a>
## 13. 実機で動かす

### 13.1 打ち込む

HEXMON のキー操作で、ダンプ（`.woz`）を 1 バイトずつ打ち込みます（『HEXMON プログラミングガイド』3.4）。

1. RST を押す（「0200」になる。アドレス入力モード）
2. A/D を押して、データ入力モードにする
3. ダンプの 1 行目の最初のバイトから、「2 桁入力 → INC」を 1 バイトずつくり返す
4. 最後のバイトを入力したら、A/D でアドレス入力モードに戻る
5. 0 2 0 0 と入力し、INC で 1 バイトずつ進めながら、ダンプと見比べる（間違いは A/D でデータ入力モードにして上書きする）

### 13.2 実行する

- 0 2 0 0 と入力して JMP を押すと、`$0200` から実行します。
- METEOR DODGE（`meteor_O2.woz`、359 バイト）は、PC の端末ソフトを 19200bps・8 ビット・パリティなし・ストップビット 1 で開いてから実行します。端末にタイトルとスコアが出ます。
- METEOR7（`meteor7_O2.woz`、268 バイト）は、端末なしで遊べます。
- 終わるとモニタへ戻ります。途中でやめるときは RST を押します（RAM の内容は残るので、打ち込み直す必要はありません）。

