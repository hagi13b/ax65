; ============================================================================
; meteor_compiled.s — meteor.ax65 を ax65 の仕様どおりにコンパイルした「想定出力」
;
; ax65 コンパイラはまだ存在しないため、仕様書（下書き）の規則に従って手で生成した。
; ターゲット：v2（W65C02S、1MHz、RAM に読み込んで実行、console = ACIA、screen なし）
; 最適化オプション：なし（-O を付けない既定の動作）
;
; 生成の規則（仕様書の該当箇所）：
;   ・構造体の配列はフィールドごとに分ける（3.9）
;   ・ローカル変数は固定アドレスに重ねて置く（4.3）。ゼロページはポインタだけ（4.2）
;   ・引数は A/X/Y、戻り値は A（1バイト）・A+X（2バイト）・キャリー（bool / Option）（5.2, 5.3）
;   ・呼び出し先が実際に壊すレジスタだけを考慮する（5.4）
;   ・末尾呼び出しは jmp にする。インライン化はしない（5.8）
;   ・グローバル変数の初期値は起動時に毎回コピーする（4.4）。init は起動処理に展開する（10B.6）
;   ・comptime if は v2 用に解決済み（screen がないので else 側だけが残る）（10.7）
;
; bench_ で始まるラベルは計測用の目印で、生成されるコードには影響しない。
; ============================================================================

; ---- ターゲット定義とI/Oのアドレス -----------------------------------------
MONITOR = $FF1F                 ; exit = monitor($FF1F)：main が終わったら飛ぶ先
IRQ_VEC = $0280                 ; irq = indirect($0280)：モニタの ROM が jmp ($0280) で飛んでくる
T1C_L   = $6004                 ; VIA タイマー1 下位（io）
T1C_H   = $6005                 ; VIA タイマー1 上位（io(out)）
ACR     = $600B                 ; VIA 補助制御レジスタ（io(out)）
IFR     = $600D                 ; VIA 割り込みフラグ（io(in)、型は flags Ifr）
IER     = $600E                 ; VIA 割り込み許可（io(out)）
ACIA_DATA   = $8000             ; hw.acia_console の DATA（io）
ACIA_STATUS = $8001             ; hw.acia_console の STATUS（io(in)）

; ---- ゼロページ（ポインタだけが自動で置かれる） ---------------------------
pr_s    = $00                   ; print の引数 s（str はポインタで渡す。2バイト）

; ---- ローカル変数の重ね合わせ（4.3）：同時に使われない関数同士で同じ番地を使う
mn_m      = LOCALS+0            ; main の for m in Mid のカウンタ（respawn が X/Y を壊すのでメモリに置く）
mn_last   = LOCALS+1            ; main の let mut last（u16、2バイト）
wf_last   = LOCALS+3            ; wait_frame の引数 last（u16、2バイト）。main の後ろに置く
um_hit    = LOCALS+3            ; update_meteors の let mut hit（wait_frame と同時には使われない）
um_m      = LOCALS+4            ; update_meteors の for m in Mid のカウンタ
rs_r      = LOCALS+5            ; respawn の let r（update_meteors の後ろに置く）
putc_c    = LOCALS+3            ; putc の引数 c（後で使うのでしまう）
bd_v      = LOCALS+3            ; bcd16_digits の引数（2バイト）

        .org $0300              ; load = $0300

; ============================================================================
; 起動処理（コンパイラが自動で作る）
; ============================================================================
start:                          ; Wozmon から 300R で実行される入口
        sei                     ; 割り込みを止める（init が終わるまで割り込みを入れない）
        cld                     ; 10進モードを解除しておく
        ldx #$FF                ; スタックポインタの初期値
        txs                     ; スタックポインタを設定する
        ldx #GLOBALS_SIZE-1     ; 初期値付きのグローバル変数のバイト数 - 1
@copy:  lda init_image,x        ; 初期値の元データを1バイト読む
        sta globals,x           ; 変数に書く（RAM で2回実行しても前回の値が残らない）
        dex                     ; 次のバイトへ
        bpl @copy               ; 0 番目まで繰り返す
        lda #<on_irq            ; IRQ 処理のアドレスの下位バイト
        sta IRQ_VEC             ; RAM 上の IRQ ベクタに書く（割り込みは止まっている）
        lda #>on_irq            ; IRQ 処理のアドレスの上位バイト
        sta IRQ_VEC+1           ; RAM 上の IRQ ベクタに書く
        ; ---- meteor.ax65 の init ブロック（ここに展開される） ----
        lda #$40                ; ACR = 0b0100_0000
        sta ACR                 ; タイマー1 を連続モードにする
        lda #$18                ; T1C_L = (T1_PERIOD & $FF)（T1_PERIOD = 16664 = $4118）
        sta T1C_L               ; 下位を先に書く（I/O の順序はソースどおり）
        lda #$41                ; T1C_H = (T1_PERIOD >> 8)
        sta T1C_H               ; 上位を書くとタイマーが動き出す
        lda #$C0                ; IER = 0b1100_0000
        sta IER                 ; タイマー1 の割り込みを許可する
        jsr main                ; main を呼ぶ
        jmp MONITOR             ; main が終わったらモニタへ戻る（exit = monitor）

; ============================================================================
; 割り込み処理：#[interrupt(irq)] fn on_irq()
; ============================================================================
on_irq:                         ; IRQ の入口（使うレジスタは A だけなので A だけ退避する）
        pha                     ; A を退避する
        bit IFR                 ; if IFR.t1：ビット6 は bit 命令で V フラグに入る
        bvc @end                ; t1 が立っていなければ終わる
        lda T1C_L               ; let _ = T1C_L：読むだけで割り込みフラグが消える
        inc TICKS               ; TICKS += 1（u16）：下位バイトを 1 増やす
        bne @end                ; 下位が 0 に戻らなければ終わり
        inc TICKS+1             ; 0 に戻ったら上位バイトに繰り上げる
@end:   pla                     ; A を元に戻す
        rti                     ; 割り込みが入る前の場所へ戻る

; ============================================================================
; fn ticks() -> u16 { atomic { TICKS } }
; ============================================================================
ticks:                          ; 戻り値 u16 は A（下位）と X（上位）
        php                     ; atomic の始まり：割り込みの許可状態を退避する
        sei                     ; 割り込みを止める
        lda TICKS               ; TICKS の下位バイトを読む
        ldx TICKS+1             ; TICKS の上位バイトを読む
        plp                     ; atomic の終わり：割り込みの許可状態を元に戻す
        rts                     ; 呼び出し元に戻る

; ============================================================================
; fn wait_frame(last: u16) -> u16
; ============================================================================
wait_frame:                     ; 引数 last は A（下位）と X（上位）で届く
        sta wf_last             ; last はループの中で何度も使うので固定領域にしまう
        stx wf_last+1           ; 上位バイトもしまう
@loop:  jsr ticks               ; let now = ticks()（now は A と X に入る）
        cmp wf_last             ; now の下位と last の下位を比べる
        bne @ret                ; 違えば now != last
        cpx wf_last+1           ; 上位どうしを比べる
        bne @ret                ; 違えば now != last
        wai                     ; comptime if target.cpu == w65c02s { wait_interrupt(); }
        bra @loop               ; loop の先頭へ戻る
@ret:   rts                     ; return now（now は A と X に入ったまま）

; ============================================================================
; asm fn rand_step(r: u8) -> u8（ソースに書いたアセンブリがそのまま入る）
; ============================================================================
rand_step:                      ; r は A で届く
        asl a                   ; A を左に1ビットずらす
        bcc @done               ; はみ出たビットが 0 なら done へ
        eor #$1D                ; 1 なら決まった値と XOR を取る
@done:  rts                     ; A を戻り値として返す

; ============================================================================
; fn rand() -> u8
; ============================================================================
rand:                           ; 壊すレジスタは A とフラグだけ（X と Y は保たれる）
        lda SEED                ; rand_step(SEED) の引数を A に入れる
        jsr rand_step           ; rand_step を呼ぶ（インライン化はしない）
        sta SEED                ; SEED = rand_step(SEED)
        rts                     ; 最後の式 SEED は A に入っている

; ============================================================================
; hw.acia_console の実装（impl console）
; ============================================================================
putc:                           ; pub fn putc(c: char)：c は A で届く
        sta putc_c              ; c は待ちループの後で使うのでしまう
@wait:  lda ACIA_STATUS         ; 送信の準備ができたか調べる
        and #$10                ; ビット4（送信バッファが空）を取り出す
        beq @wait               ; まだなら待つ
        lda putc_c              ; しまっておいた c を読む
        sta ACIA_DATA           ; 1文字送る
        rts                     ; 呼び出し元に戻る（壊すのは A とフラグだけ）

poll:                           ; pub fn poll() -> Option<char>
        lda ACIA_STATUS         ; 受信したか調べる
        and #$08                ; ビット3（受信データあり）を取り出す
        beq @none               ; なければ None へ
        lda ACIA_DATA           ; 受信した文字を A に読む
        sec                     ; Some（キャリー 1）
        rts                     ; 値は A に入っている
@none:  clc                     ; None（キャリー 0）
        rts                     ; 呼び出し元に戻る

; ============================================================================
; fn read_key() -> Option<Key>（Key：Left=0、Right=1、Quit=2）
; ============================================================================
read_key:                       ; 戻り値：キャリー＝有無、A＝Key
        jsr poll                ; let c = poll()?
        bcc @ret                ; ? 演算子：None ならキャリー 0 のまま抜ける
        cmp #'A'                ; match c：'A' か
        beq @left               ; 'A' なら Left へ
        cmp #'a'                ; 'a' か
        beq @left               ; 'a' なら Left へ
        cmp #'D'                ; 'D' か
        beq @right              ; 'D' なら Right へ
        cmp #'d'                ; 'd' か
        beq @right              ; 'd' なら Right へ
        cmp #'Q'                ; 'Q' か
        beq @quit               ; 'Q' なら Quit へ
        cmp #'q'                ; 'q' か
        beq @quit               ; 'q' なら Quit へ
        clc                     ; _ => None（キャリー 0）
@ret:   rts                     ; 呼び出し元に戻る
@left:  lda #0                  ; Key::Left
        sec                     ; Some（キャリー 1）
        rts                     ; 呼び出し元に戻る
@right: lda #1                  ; Key::Right
        sec                     ; Some（キャリー 1）
        rts                     ; 呼び出し元に戻る
@quit:  lda #2                  ; Key::Quit
        sec                     ; Some（キャリー 1）
        rts                     ; 呼び出し元に戻る

; ============================================================================
; fn move_player(k: Key)
; ============================================================================
move_player:                    ; k は A で届く
        cmp #0                  ; match k：Key::Left か
        beq @left               ; Left なら @left へ
        cmp #1                  ; Key::Right か
        beq @right              ; Right なら @right へ
        rts                     ; Key::Quit => {}
@left:  lda PLAYER_X            ; PLAYER_X.saturating_sub(1)：PLAYER_X を読む
        sec                     ; 引き算の準備
        sbc #1                  ; 1 を引く
        bcs @lst                ; 借りがなければ（0 未満にならなければ）そのまま
        lda #0                  ; 0 未満になったら 0 で止める
@lst:   sta PLAYER_X            ; PLAYER_X に書く
        rts                     ; 呼び出し元に戻る
@right: lda PLAYER_X            ; if PLAYER_X < FIELD_W - 1：PLAYER_X を読む
        cmp #19                 ; 19 と比べる
        bcs @rret               ; 19 以上なら何もしない
        inc PLAYER_X            ; PLAYER_X += 1
@rret:  rts                     ; 呼び出し元に戻る

; ============================================================================
; fn clear_meteors()
; ============================================================================
clear_meteors:                  ; #[reg(x)]：m を X に置く
        ldx #0                  ; for m in Mid：m = 0 から
@loop:  stz M_Y,x               ; METEORS[m].y = 0
        txa                     ; m as u8 を A に入れる（stx に abs,x のモードはない）
        sta M_WAIT,x            ; METEORS[m].wait = m as u8
        inx                     ; 次の m へ
        cpx #8                  ; Mid の終わり（8）に達したか
        bne @loop               ; 達していなければ繰り返す
        rts                     ; 呼び出し元に戻る

; ============================================================================
; fn respawn(m: Mid)
; ============================================================================
respawn:                        ; m は A で届く。rand は X を壊さないので m は X に置ける
        tax                     ; m を X に移す
        jsr rand                ; let r = rand()（A が壊れるだけ）
        sta rs_r                ; r は2回使うのでしまう（X は m、Y は表引きに使う）
        and #7                  ; Mid::wrap(r)：下位3ビットだけを取る
        tay                     ; 表の添字を Y に入れる
        lda SPAWN_X,y           ; SPAWN_X[Mid::wrap(r)]
        sta M_X,x               ; METEORS[m].x = …
        stz M_Y,x               ; METEORS[m].y = 0
        lda rs_r                ; r を読み直す
        and #3                  ; r & 3
        sta M_WAIT,x            ; METEORS[m].wait = r & 3
        rts                     ; 呼び出し元に戻る

; ============================================================================
; fn update_meteors() -> bool（戻り値 bool はキャリー）
; ============================================================================
update_meteors:                 ; ループの中で respawn（X と Y を壊す）を呼ぶので、m はメモリに置く
        stz um_hit              ; let mut hit = false
        stz um_m                ; for m in Mid：m = 0 から
@loop:  ldx um_m                ; m を X に読む（添字に使う）
        lda M_WAIT,x            ; if METEORS[m].wait > 0
        beq @fall               ; 0 なら落下の処理へ
        dec M_WAIT,x            ; METEORS[m].wait -= 1
        bra @next               ; continue
@fall:  lda #3                  ; METEORS[m].wait = 3
        sta M_WAIT,x            ; 書く
        inc M_Y,x               ; METEORS[m].y += 1
        lda M_Y,x               ; if METEORS[m].y == FIELD_H
        cmp #16                 ; FIELD_H = 16 と比べる
        bne @chk                ; 違えば else if へ
        txa                     ; respawn(m)：m を A に入れる
        jsr respawn             ; respawn を呼ぶ（X と Y が壊れる）
        sed                     ; SCORE += 1（bcd16）：10進モードにする
        clc                     ; 足し算の準備
        lda SCORE               ; SCORE の下位2桁を読む
        adc #$01                ; 1 を足す（BCD の定数 1 = $01）
        sta SCORE               ; 下位2桁を書く
        lda SCORE+1             ; 上位2桁を読む
        adc #$00                ; 繰り上がりを足す
        sta SCORE+1             ; 上位2桁を書く
        cld                     ; 10進モードを解除する
        bra @next               ; 次の m へ
@chk:   cmp #15                 ; else if METEORS[m].y == PLAYER_Y（A には y が残っている）
        bne @next               ; 違えば次の m へ
        lda M_X,x               ; && METEORS[m].x == PLAYER_X
        cmp PLAYER_X            ; PLAYER_X と比べる
        bne @next               ; 違えば次の m へ
        lda #1                  ; hit = true
        sta um_hit              ; 書く
@next:  inc um_m                ; m を 1 増やす
        lda um_m                ; m を読む
        cmp #8                  ; Mid の終わり（8）に達したか
        bne @loop               ; 達していなければ繰り返す
        lda um_hit              ; hit（0 か 1）を読む
        cmp #1                  ; 1 ならキャリー 1、0 ならキャリー 0 になる
        rts                     ; bool をキャリーで返す

; ============================================================================
; fn print(s: str)
; ============================================================================
print:                          ; s（長さ付き文字列のポインタ）はゼロページの pr_s に届く
        ldy #0                  ; 先頭（長さ）の位置
        lda (pr_s),y            ; 長さを読む
        beq @done               ; 長さ 0 なら何もしない
        tax                     ; 残りの文字数を X に置く（putc は X と Y を壊さない）
@loop:  iny                     ; 次の文字の位置へ
        lda (pr_s),y            ; for c in s：1文字読む
        jsr putc                ; putc(c)
        dex                     ; 残りの文字数を 1 減らす
        bne @loop               ; 残っていれば繰り返す
@done:  rts                     ; 呼び出し元に戻る

; ============================================================================
; 標準ライブラリ bcd：SCORE.digits()（4桁を結果領域に置く）
; ============================================================================
bcd16_digits:                   ; 引数 bcd16 は A（下位）と X（上位）で届く
        sta bd_v                ; 下位をしまう
        stx bd_v+1              ; 上位をしまう
        lda bd_v+1              ; 千の位と百の位
        lsr a                   ; 上位4ビットを下へずらす（1回目）
        lsr a                   ; （2回目）
        lsr a                   ; （3回目）
        lsr a                   ; （4回目）→ 千の位
        sta DIGITS              ; 結果領域の 0 番目
        lda bd_v+1              ; もう一度読む
        and #$0F                ; 下位4ビット → 百の位
        sta DIGITS+1            ; 結果領域の 1 番目
        lda bd_v                ; 十の位と一の位
        lsr a                   ; 上位4ビットを下へずらす（1回目）
        lsr a                   ; （2回目）
        lsr a                   ; （3回目）
        lsr a                   ; （4回目）→ 十の位
        sta DIGITS+2            ; 結果領域の 2 番目
        lda bd_v                ; もう一度読む
        and #$0F                ; 下位4ビット → 一の位
        sta DIGITS+3            ; 結果領域の 3 番目
        rts                     ; 呼び出し元に戻る

; ============================================================================
; fn print_score()
; ============================================================================
print_score:                    ; for d in SCORE.digits()
        lda SCORE               ; SCORE の下位を A に
        ldx SCORE+1             ; 上位を X に
        jsr bcd16_digits        ; 4桁を結果領域 DIGITS に作る
        ldx #0                  ; 桁の番号（putc は X を壊さないので X に置ける）
@loop:  lda DIGITS,x            ; d を読む
        clc                     ; '0' as u8 + d：足し算の準備
        adc #'0'                ; '0' を足して数字の文字にする
        jsr putc                ; putc(…)
        inx                     ; 次の桁へ
        cpx #4                  ; 4桁出したか
        bne @loop               ; まだなら繰り返す
        rts                     ; 呼び出し元に戻る

; ============================================================================
; fn draw()（v2 では screen が bind されていないので else 側だけが残る）
; ============================================================================
draw:                           ; 画面のないターゲットの表示
        lda #$0D                ; putc('\r')
        jsr putc                ; 行の先頭に戻る
        lda #<S_SCORE           ; print(p"SCORE ")：文字列のアドレスの下位
        sta pr_s                ; print の引数領域（ゼロページ）に書く
        lda #>S_SCORE           ; 上位
        sta pr_s+1              ; 書く
        jsr print               ; print を呼ぶ
        jmp print_score         ; print_score()（末尾呼び出しなので jmp）

; ============================================================================
; fn main()
; ============================================================================
main:                           ; プログラム本体
        lda #<TITLE             ; print(TITLE)：アドレスの下位
        sta pr_s                ; 引数領域に書く
        lda #>TITLE             ; 上位
        sta pr_s+1              ; 書く
        jsr print               ; print を呼ぶ
        lda #$0D                ; putc('\r')
        jsr putc                ; 改行する
        jsr clear_meteors       ; clear_meteors()
        stz mn_m                ; for m in Mid：m = 0 から（respawn が X/Y を壊すのでメモリ）
main_rs:
        lda mn_m                ; respawn(m)：m を A に入れる
        jsr respawn             ; respawn を呼ぶ
        inc mn_m                ; 次の m へ
        lda mn_m                ; m を読む
        cmp #8                  ; 8 に達したか
        bne main_rs             ; 達していなければ繰り返す
        cli                     ; irq_enable()
        jsr ticks               ; let mut last = ticks()
        sta mn_last             ; last の下位をしまう
        stx mn_last+1           ; last の上位をしまう
bench_loop:                     ; 'game: loop の先頭
        lda mn_last             ; wait_frame(last)：下位を A に
        ldx mn_last+1           ; 上位を X に
        jsr wait_frame          ; 次のフレームまで待つ
        sta mn_last             ; last = …：下位をしまう
        stx mn_last+1           ; 上位をしまう
bench_frame_begin:              ; ---- ここからフレームの処理 ----
        jsr read_key            ; if let Some(k) = read_key()
        bcc bench_upd           ; None なら入力の処理を飛ばす
        cmp #2                  ; if k == Key::Quit
        beq main_over           ; break 'game
        jsr move_player         ; move_player(k)（k は A に入ったまま）
bench_upd:                      ; ---- 隕石の更新 ----
        jsr update_meteors      ; if update_meteors()
        bcs main_over           ; true なら break 'game
bench_draw:                     ; ---- 表示 ----
        jsr draw                ; draw()
bench_frame_end:                ; ---- フレームの処理の終わり ----
        bra bench_loop          ; 'game: loop の先頭へ戻る
main_over:                      ; 'game: loop の後ろ
        lda #<S_OVER            ; print(p"\rGAME OVER  SCORE ")：アドレスの下位
        sta pr_s                ; 引数領域に書く
        lda #>S_OVER            ; 上位
        sta pr_s+1              ; 書く
        jsr print               ; print を呼ぶ
        jsr print_score         ; print_score()
        lda #$0D                ; putc('\r')
        jmp putc                ; 末尾呼び出しなので jmp（putc の rts で起動処理へ戻る）
code_end:

; ============================================================================
; 定数（const）
; ============================================================================
SPAWN_X:                        ; const SPAWN_X: u8[Mid]（comptime fn spawn_x で計算済み）
        .byte 3, 10, 17, 4, 11, 18, 5, 12
TITLE:                          ; const TITLE: str = p"METEOR DODGE"
        .byte 12, "METEOR DODGE"
S_SCORE:                        ; p"SCORE "
        .byte 6, "SCORE "
S_OVER:                         ; p"\rGAME OVER  SCORE "
        .byte 18, $0D, "GAME OVER  SCORE "
init_image:                     ; グローバル変数の初期値の元データ（globals と同じ並び）
        .byte 10                ; PLAYER_X = FIELD_W / 2
        .byte $00, $00          ; SCORE = 0（bcd16）
        .byte $5A               ; SEED = $5A
        .byte $00, $00          ; TICKS = 0（u16）
        .byte 0,0,0,0,0,0,0,0   ; METEORS[*].x
        .byte 0,0,0,0,0,0,0,0   ; METEORS[*].y
        .byte 0,0,0,0,0,0,0,0   ; METEORS[*].wait
data_end:

; ============================================================================
; 変数（RAM、コードの後ろ）
; ============================================================================
globals:                        ; 初期値付きのグローバル変数（30バイト）
PLAYER_X: .res 1                ; static mut PLAYER_X: u8
SCORE:    .res 2                ; static mut SCORE: bcd16
SEED:     .res 1                ; static mut SEED: u8
TICKS:    .res 2                ; shared(irq) TICKS: u16
M_X:      .res 8                ; METEORS[*].x（フィールドごとの配列）
M_Y:      .res 8                ; METEORS[*].y
M_WAIT:   .res 8                ; METEORS[*].wait
GLOBALS_SIZE = 30               ; 初期値をコピーするバイト数
LOCALS:   .res 6                ; ローカル変数の重ね合わせ領域（最大 6 バイト）
DIGITS:   .res 4                ; bcd16_digits の結果領域
vars_end:
