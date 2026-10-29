; ============================================================================
; meteor_hand.s — 同じ仕様のゲームを、最初からアセンブリで手書きしたもの
;
; ゲームの仕様（meteor_game_spec.md）から書いた。ターゲットは v2（W65C02S、1MHz、RAM で実行、ACIA）。
; 端末への出力（文字の並び）は、コンパイル版（meteor_O0〜Os.s）とまったく同じになるように書いている。
;
; 手書きならではの工夫：
;   ・よく使う変数と隕石の配列をすべてゼロページに置く
;   ・乱数、出し直し、入力の判定などを呼び出さずにその場に書く（インライン）
;   ・割り込み処理はレジスタを使わない（bit 命令でフラグを調べて消す）
;   ・TICKS は 8 ビットだけにする（フレームの切り替わりがわかれば十分）
;   ・文字列は 0 終端で1つの表にまとめ、X に表の位置を入れて渡す
;   ・putc は bit 命令で待つので、文字を A に入れたまま待てる
;   ・大文字・小文字は ora #$20 でまとめて判定する
; bench_ で始まるラベルは計測用の目印で、コードには影響しない。
; ============================================================================

MONITOR = $FF1F                 ; main が終わったら飛ぶ先（モニタの入口）
IRQ_VEC = $0280                 ; モニタの ROM が jmp ($0280) で飛んでくる場所
T1C_L   = $6004                 ; VIA タイマー1 下位
T1C_H   = $6005                 ; VIA タイマー1 上位
ACR     = $600B                 ; VIA 補助制御レジスタ
IFR     = $600D                 ; VIA 割り込みフラグ
IER     = $600E                 ; VIA 割り込み許可
ACIA_DATA   = $8000             ; ACIA のデータ
ACIA_STATUS = $8001             ; ACIA の状態

; ---- ゼロページの変数 --------------------------------------------------------
player_x = $00                  ; プレイヤーの列
score    = $01                  ; スコア（BCD、2バイト：$01 が下位、$02 が上位）
seed     = $03                  ; 乱数の種
ticks    = $04                  ; 1/60 秒ごとに増える（8ビット）
last     = $05                  ; 前のフレームの ticks
hit      = $06                  ; このフレームで当たったら 0 以外
mx       = $10                  ; 隕石の x（8個）
my       = $18                  ; 隕石の y（8個）
mw       = $20                  ; 隕石の wait（8個）

        .org $0300              ; 読み込み先

; ============================================================================
; 起動
; ============================================================================
start:                          ; Wozmon から 300R で実行される入口
        sei                     ; 割り込みを止める
        cld                     ; 10進モードを解除する
        ldx #$FF                ; スタックポインタの初期値
        txs                     ; スタックポインタを設定する
        lda #10                 ; プレイヤーを中央に置く
        sta player_x            ; player_x = 10
        stz score               ; スコアの下位を 0 にする
        stz score+1             ; スコアの上位を 0 にする
        lda #$5A                ; 乱数の種
        sta seed                ; seed = $5A
        stz ticks               ; ticks = 0
        lda #<irq               ; IRQ 処理のアドレスの下位
        sta IRQ_VEC             ; RAM 上の IRQ ベクタに書く
        lda #>irq               ; 上位
        sta IRQ_VEC+1           ; 書く
        lda #$40                ; タイマー1 を連続モードにする値
        sta ACR                 ; 書く
        lda #$18                ; 周期 $4118 の下位
        sta T1C_L               ; 下位を先に書く
        lda #$41                ; 上位
        sta T1C_H               ; 上位を書くとタイマーが動き出す
        lda #$C0                ; タイマー1 の割り込みを許可する値
        sta IER                 ; 書く
        ldx #S_TITLE            ; タイトルの文字列の位置
        jsr prstr               ; タイトルを出す
        lda #$0D                ; 改行
        jsr putc                ; 出す
        ldx #0                  ; 隕石の番号 0 から
@init:  jsr respawn             ; X 番目の隕石を出す（X は保たれる）
        inx                     ; 次の隕石へ
        cpx #8                  ; 8個出したか
        bne @init               ; まだなら繰り返す
        cli                     ; 割り込みを許可する
        lda ticks               ; 今の ticks を
        sta last                ; last に覚える

; ============================================================================
; フレームのループ
; ============================================================================
bench_loop:                     ; ループの先頭
        lda ticks               ; 今の ticks を読む
        cmp last                ; 前のフレームと比べる
        bne frame_go            ; 変わっていれば次のフレームへ進む
        wai                     ; 変わっていなければ割り込みまで CPU を止める
        bra bench_loop          ; もう一度調べる
frame_go:
        sta last                ; 今の ticks を覚える
bench_frame_begin:              ; ---- ここからフレームの処理 ----
        lda ACIA_STATUS         ; キーが届いたか調べる
        and #$08                ; 受信データありのビット
        beq bench_upd           ; なければ入力の処理を飛ばす
        lda ACIA_DATA           ; 届いた文字を読む
        ora #$20                ; 英大文字を小文字にそろえる（'A'→'a'）
        cmp #'a'                ; 左か
        beq key_left            ; 左なら key_left へ
        cmp #'d'                ; 右か
        beq key_right           ; 右なら key_right へ
        cmp #'q'                ; 終了か
        beq game_over           ; 終了なら game_over へ
        bra bench_upd           ; それ以外は無視する
key_left:
        lda player_x            ; プレイヤーの列を読む
        beq bench_upd           ; 0 ならそれ以上左へ行かない
        dec player_x            ; 1 列左へ
        bra bench_upd           ; 更新へ
key_right:
        lda player_x            ; プレイヤーの列を読む
        cmp #19                 ; 右端（19）と比べる
        bcs bench_upd           ; 右端ならそれ以上右へ行かない
        inc player_x            ; 1 列右へ
bench_upd:                      ; ---- 隕石の更新 ----
        stz hit                 ; 当たりの印を消す
        ldx #0                  ; 隕石の番号 0 から
upd_loop:
        lda mw,x                ; 待ち時間を読む
        beq upd_fall            ; 0 なら落とす
        dec mw,x                ; 待ち時間を 1 減らす
        bra upd_next            ; 次の隕石へ
upd_fall:
        lda #3                  ; 次に落ちるまでの待ち時間
        sta mw,x                ; 書く
        inc my,x                ; 1 マス落とす
        lda my,x                ; 落ちた後の y を読む
        cmp #16                 ; 画面の下（16）に達したか
        bne upd_chk             ; 達していなければ当たりの判定へ
        jsr respawn             ; 上から出し直す（X は保たれる）
        sed                     ; 10進モードにする
        lda score               ; スコアの下位を読む
        clc                     ; 足し算の準備
        adc #$01                ; 1 点足す
        sta score               ; 書く
        bcc @nocarry            ; 繰り上がりがなければ上位はそのまま
        lda score+1             ; 上位を読む
        adc #$00                ; 繰り上がり（キャリー）を足す
        sta score+1             ; 書く
@nocarry:
        cld                     ; 10進モードを解除する
        bra upd_next            ; 次の隕石へ
upd_chk:
        cmp #15                 ; プレイヤーの行（15）か
        bne upd_next            ; 違えば次の隕石へ
        lda mx,x                ; 隕石の x を読む
        cmp player_x            ; プレイヤーの列と比べる
        bne upd_next            ; 違えば次の隕石へ
        dec hit                 ; 当たりの印を付ける（0 → $FF）
upd_next:
        inx                     ; 次の隕石へ
        cpx #8                  ; 8個処理したか
        bne upd_loop            ; まだなら繰り返す
        lda hit                 ; 当たりの印を読む
        bne game_over           ; 当たっていればゲームオーバーへ
bench_draw:                     ; ---- 表示 ----
        lda #$0D                ; 行の先頭へ
        jsr putc                ; 出す
        ldx #S_SCORE            ; "SCORE " の位置
        jsr prstr               ; 出す
        jsr prscore             ; スコアを4桁で出す
bench_frame_end:                ; ---- フレームの処理の終わり ----
        jmp bench_loop          ; 次のフレームへ（遠いので bra は届かない）

game_over:
        ldx #S_OVER             ; "\rGAME OVER  SCORE " の位置
        jsr prstr               ; 出す
        jsr prscore             ; スコアを出す
        lda #$0D                ; 改行
        jsr putc                ; 出す
        jmp MONITOR             ; モニタへ戻る

; ============================================================================
; サブルーチン
; ============================================================================
respawn:                        ; X 番目の隕石を一番上に出し直す（X と Y は保たれる）
        lda seed                ; 乱数の種を読む
        asl a                   ; 左に1ビットずらす
        bcc @noxor              ; はみ出たビットが 0 なら XOR しない
        eor #$1D                ; 1 なら決まった値と XOR を取る
@noxor: sta seed                ; 新しい種を書く
        pha                     ; 乱数を後でもう一度使うので退避する
        and #7                  ; 下位3ビット（0〜7）
        tay                     ; 表の添字にする
        lda spawn_x,y           ; 出現位置の表を引く
        sta mx,x                ; 隕石の x にする
        stz my,x                ; y = 0
        pla                     ; 乱数を戻す
        and #3                  ; 下位2ビット（0〜3）
        sta mw,x                ; 待ち時間にする
        rts                     ; 戻る（Y は壊れるが、呼び出し側は Y を使っていない）

putc:                           ; A の文字を ACIA に送る（X と Y は保たれる）
        pha                     ; 文字を退避する
        lda #$10                ; 送信バッファが空のビット
@wait:  bit ACIA_STATUS         ; ビットが立っているか調べる（A は変わらない）
        beq @wait               ; 立っていなければ待つ
        pla                     ; 文字を戻す
        sta ACIA_DATA           ; 送る
        rts                     ; 戻る

prstr:                          ; strings の X 番目から 0 の手前まで出す
        lda strings,x           ; 1文字読む
        beq @done               ; 0 なら終わり
        jsr putc                ; 出す（X は保たれる）
        inx                     ; 次の文字へ
        bra prstr               ; 繰り返す
@done:  rts                     ; 戻る

prscore:                        ; スコアを4桁の数字で出す
        lda score+1             ; 上位2桁
        jsr prbyte              ; 2桁出す
        lda score               ; 下位2桁
prbyte:                         ; A の BCD 2桁を出す（prscore から続けて実行される）
        pha                     ; 下の桁のために退避する
        lsr a                   ; 上位4ビットを下へ（1回目）
        lsr a                   ; （2回目）
        lsr a                   ; （3回目）
        lsr a                   ; （4回目）
        ora #'0'                ; 数字の文字にする
        jsr putc                ; 出す
        pla                     ; 戻す
        and #$0F                ; 下位4ビット
        ora #'0'                ; 数字の文字にする
        jmp putc                ; 出して戻る（末尾なので jmp）

irq:                            ; IRQ 処理（レジスタを使わないので退避もしない）
        bit IFR                 ; タイマー1 のビット6 を V フラグで調べる
        bvc @end                ; タイマー1 でなければ終わる
        bit T1C_L               ; 読むだけで割り込みフラグを消す（A は変わらない）
        inc ticks               ; ticks を 1 増やす
@end:   rti                     ; 戻る
code_end:

; ============================================================================
; 定数
; ============================================================================
spawn_x:                        ; 出現位置の表
        .byte 3, 10, 17, 4, 11, 18, 5, 12
strings:                        ; 0 終端の文字列をまとめた表
S_TITLE = 0                     ; "METEOR DODGE" の位置
        .byte "METEOR DODGE", 0
S_SCORE = 13                    ; "SCORE " の位置
        .byte "SCORE ", 0
S_OVER = 20                     ; "\rGAME OVER  SCORE " の位置
        .byte $0D, "GAME OVER  SCORE ", 0
data_end:
vars_end:
