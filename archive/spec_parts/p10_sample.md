
---

<a id="appD"></a>
## 付録D サンプル「METEOR DODGE」（参考）

AX65 のサンプルのゲームです。詳細は別冊『AX65 サンプルと効果測定 1.1』（`ax65_1.1/sample/ax65_sample_1.1.md`）にあります。

| 項目 | 内容 |
|---|---|
| 内容 | 上から落ちてくる 8 個の隕石を、一番下のプレイヤーが左右に動いてよけるゲーム |
| ターゲット | v2（実機。スコアをシリアル端末に表示）、v3（仮。v2 に画面を足した構成。隕石とプレイヤーを画面に描く） |
| 操作 | HEXPAD の 4 で左、6 で右、A/D で終了 |
| 測定のターゲット | v2（W65C02S、約 1.8425MHz、割り込みなし。フレームの時間はタイマー T2 で計る） |
| 7セグメント版 | METEOR7（v2 の HEXPAD だけで遊べる。268 バイト） |

### D.1 フォルダの構成

```
sample/meteor/
├── ax65.proj                    プロジェクトファイル（17.2節）
├── targets/
│   ├── v2.target                実機の構成
│   └── v3.target                v2 を引き継ぎ、画面を追加（extends）
├── src/
│   └── meteor.ax65              ゲームの本体（main を含む）
└── hw/
    ├── acia_console.ax65        impl console（UART）
    ├── hexpad.ax65              HEXPAD のキー
    ├── frame_timer.ax65         タイマー T2 で 1/60 秒を計る
    ├── hexmon_keyboard.ax65     impl keyboard（HEXMON の POLL_KEY）
    └── v3_screen.ax65           impl screen（v3 だけ）
```

### D.2 使っている主な機能

| 機能 | サンプルでの使い方 | 節 |
|---|---|---|
| 範囲型 | `range Mid = 0..8;` を隕石の配列の添字にする（範囲の検査が不要） | [3.4](#s3-4) |
| 列挙型、`match` | `enum Key { Left, Right, Quit }` と、網羅性の検査 | [3.5](#s3-5)、[6.6](#s6-6) |
| ビットフラグ型 | 6522 の割り込みフラグ `flags Ifr`、UART の状態 `flags AciaStatus` | [3.6](#s3-6) |
| 構造体の配列 | `[Meteor; Mid]`（フィールドごとの配列になる） | [3.7](#s3-7)、[3.8](#s3-8) |
| 文字列、変換表 | `p"METEOR DODGE"`、`'*'@screen` | [2.7](#s2-7)、[3.3](#s3-3) |
| `Option` と `?` | `fn read_key() -> Option<Key>` と `hexpad::poll_key()?` | [3.11](#s3-11) |
| BCD | `static SCORE: bcd16`、`SCORE += 1;`、`SCORE.digits()` | [5.5](#s5-5) |
| コンパイル時の計算 | `comptime fn spawn_x`、`[for i in 0..8 => spawn_x(i)]`、`comptime assert` | [7.3](#s7-3)、[13.3](#s13-3)、[13.5](#s13-5) |
| `asm fn`、`extern fn` | 乱数の `rand_step`。HEXMON の POLL_KEY を keyboard の実装にする `hw::hexmon_keyboard`（v2.target が bind する）。METEOR7 では DELAY_MS | [7.4](#s7-4)、[7.6](#s7-6) |
| インターフェースと bind | `console`・`screen` を `use` し、実装はターゲット定義の `bind` で選ぶ | [8.4](#s8-4)、[16.6](#s16-6) |
| `init` | UART のコマンドレジスタの設定 | [8.5](#s8-5) |
| I/O | `io`・`io(in)`・`io(out)`、タイマー T2 の下位・上位の順序 | [11章](#s11) |
| ターゲット定義 | `exit = rts`、`irq = none`、モニタが使う T1・ACR の `reserve`、`extends` | [16章](#s16) |
| 条件コンパイル | `comptime if target.has(screen)` | [13.6](#s13-6) |
| ループ | `#[reg(x)] for m in Mid`、`(0..FIELD_W).rev()`、ラベル付きの `break` | [6.4](#s6-4)、[6.5](#s6-5) |

割り込み（`#[interrupt]`、`shared`、`atomic`）は、v2 では使えないので、このサンプルでは使っていません。

### D.3 測定の結果

v2 のターゲットで 600 フレーム動かした結果です。「1 フレーム」は、送信待ちのない理想の条件での平均のサイクル数です。

| 段階 | 1 フレーム（サイクル） | コード（バイト） |
|---|---|---|
| `-O0` | 1058.0 | 493 |
| `-O1` | 807.5 | 368 |
| **`-O2`（既定）** | **761.9** | **312** |
| `-Os` | 737.5 | 352 |
| 手書きのアセンブリ | 734.3 | 282 |
| cc65 v2.18（`-Oirs -Cl --codesize 500`。C 版） | 2148.5 | 821 |

- AX65 の各段階のコンパイル結果は、この仕様書の規則に従って手で生成した想定出力です。
- cc65 は、5 つの設定のうち最速のものです。AX65 の `-O2` に対して、1 フレームは 2.82 倍、コードは 2.63 倍でした。端末への出力は AX65 版と一致しています。
- 測定の方法、シリアル端末の条件での結果、考察、cc65 の全設定の結果は、別冊にあります。
