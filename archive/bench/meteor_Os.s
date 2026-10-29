; ============================================================================
; meteor_Os.s — meteor.ax65 を -Os（速さ優先）でコンパイルした「想定出力」（仕様 版0.2）
;
; ax65 コンパイラはまだ存在しないため、仕様の規則に従って手で生成した。ターゲットは v2。
; -Os で行ったこと（仕様書 12.3）：-O2 のすべて（ただし共通部分のくくり出しはしない）に加えて、
;   ・2か所から呼ばれる小さな関数 respawn も、ループの中で呼ばれているので展開した
;     （meteor.ax65 では respawn に #[cold] を付けていないため）
;   ・for で直接回す SCORE.digits() を展開した
;   ・何度も回るループ（隕石の更新）がページ境界をまたがないように配置した
; ============================================================================

MONITOR = $FF1F                 ; exit = monitor($FF1F)
IRQ_VEC = $0280                 ; irq = indirect($0280)
T1C_L   = $6004                 ; VIA タイマー1 下位
T1C_H   = $6005                 ; VIA タイマー1 上位
ACR     = $600B                 ; VIA 補助制御レジスタ
IFR     = $600D                 ; VIA 割り込みフラグ
IER     = $600E                 ; VIA 割り込み許可
ACIA_DATA   = $8000             ; ACIA のデータ
ACIA_STATUS = $8001             ; ACIA の状態

; ---- ゼロページ（-O の自動配置） --------------------------------------------
pr_s      = $00                 ; print の引数 s（ポインタ、2バイト）
mn_last   = $02                 ; main の last（u16）
rs_r      = $04                 ; respawn を展開した部分の r
um_hit    = $05                 ; update_meteors を展開した部分の hit
putc_c    = $06                 ; putc の引数 c
PLAYER_X  = $30                 ; static mut PLAYER_X
SCORE     = $31                 ; static mut SCORE（2バイト）
SEED      = $33                 ; static mut SEED
TICKS     = $34                 ; shared(irq) TICKS（2バイト）
M_X       = $36                 ; METEORS[*].x（8個）
M_Y       = $3E                 ; METEORS[*].y（8個）
M_WAIT    = $46                 ; METEORS[*].wait（8個）

        .org $0300              ; load = $0300
        jmp start               ; ループの配置のため、サブルーチンを先に置いたので起動処理へ飛ぶ
                                ; （-Os：隕石の更新のループがページ境界をまたがないように並び順を入れ替えた）

; ============================================================================
; 割り込み処理
; ============================================================================
on_irq:                         ; 改善案(1)：レジスタを使わないので退避もしない
        bit IFR                 ; if IFR.t1（ビット6 → V フラグ）
        bvc @end                ; 立っていなければ終わる
        bit T1C_L               ; let _ = T1C_L を bit で読む（A は変わらない）
        inc TICKS               ; TICKS += 1：下位
        bne @end                ; 0 に戻らなければ終わり
        inc TICKS+1             ; 上位に繰り上げる
@end:   rti                     ; 戻る

; ============================================================================
; 残した関数（複数の場所から呼ばれる）
; ============================================================================
putc:                           ; c は A で届く
        sta putc_c              ; c をしまう（zp）
@wait:  lda ACIA_STATUS         ; 送信の準備ができたか
        and #$10                ; 送信バッファが空のビット
        beq @wait               ; まだなら待つ
        lda putc_c              ; c を読む
        sta ACIA_DATA           ; 送る
        rts                     ; 戻る

print:                          ; s のアドレスは A（下位）と X（上位）で届く（5.2節）
        sta pr_s                ; 入口でゼロページのポインタに書く
        stx pr_s+1              ; 上位も書く
        ldy #0                  ; 長さの位置
        lda (pr_s),y            ; 長さを読む
        beq @done               ; 0 なら何もしない
        tax                     ; 残りの文字数を X に
@loop:  iny                     ; 次の文字の位置へ
        lda (pr_s),y            ; 1文字読む
        jsr putc                ; 出す
        dex                     ; 残りを 1 減らす
        bne @loop               ; 残っていれば繰り返す
@done:  rts                     ; 戻る

print_score:                    ; 改善案(3)：for d in SCORE.digits() を結果領域なしで展開
        lda SCORE+1             ; 千の位：上位2桁を読む
        lsr a                   ; 上位4ビットを下へ（1回目）
        lsr a                   ; （2回目）
        lsr a                   ; （3回目）
        lsr a                   ; （4回目）
        clc                     ; '0' as u8 + d
        adc #'0'                ; 数字の文字にする
        jsr putc                ; 出す
        lda SCORE+1             ; 百の位：上位2桁を読む
        and #$0F                ; 下位4ビット
        clc                     ; '0' as u8 + d
        adc #'0'                ; 数字の文字にする
        jsr putc                ; 出す
        lda SCORE               ; 十の位：下位2桁を読む
        lsr a                   ; 上位4ビットを下へ（1回目）
        lsr a                   ; （2回目）
        lsr a                   ; （3回目）
        lsr a                   ; （4回目）
        clc                     ; '0' as u8 + d
        adc #'0'                ; 数字の文字にする
        jsr putc                ; 出す
        lda SCORE               ; 一の位：下位2桁を読む
        and #$0F                ; 下位4ビット
        clc                     ; '0' as u8 + d
        adc #'0'                ; 数字の文字にする
        jmp putc                ; 出して戻る（末尾呼び出し）

; ============================================================================
; 起動処理（main を展開している）
; ============================================================================
start:                          ; Wozmon から 300R で実行される入口
        sei                     ; 割り込みを止める
        cld                     ; 10進モードを解除する
        ldx #$FF                ; スタックポインタの初期値
        txs                     ; 設定する
        lda #10                 ; 改善案(2)：初期値が 0 でない変数だけ即値で書く（PLAYER_X = 10）
        sta PLAYER_X            ; 書く
        lda #$5A                ; SEED = $5A
        sta SEED                ; 書く
        stz SCORE               ; 初期値が 0 の変数は stz で埋める（SCORE 下位）
        stz SCORE+1             ; SCORE 上位
        stz TICKS               ; TICKS 下位
        stz TICKS+1             ; TICKS 上位
        ldx #23                 ; METEORS の 24 バイト（x、y、wait）をまとめて 0 にする
@zero:  stz M_X,x               ; 0 を書く
        dex                     ; 次のバイトへ
        bpl @zero               ; 0 番目まで繰り返す
        lda #<on_irq            ; IRQ 処理のアドレスの下位
        sta IRQ_VEC             ; RAM 上の IRQ ベクタに書く
        lda #>on_irq            ; 上位
        sta IRQ_VEC+1           ; 書く
        lda #$40                ; init：ACR = 0b0100_0000
        sta ACR                 ; タイマー1 を連続モードにする
        lda #$18                ; init：T1C_L = $18
        sta T1C_L               ; 下位を先に書く
        lda #$41                ; init：T1C_H = $41
        sta T1C_H               ; 上位を書くと動き出す
        lda #$C0                ; init：IER = 0b1100_0000
        sta IER                 ; タイマー1 の割り込みを許可する
        ; ---- ここから main（展開済み） ----
        lda #<TITLE           ; 文字列のアドレスの下位を A に（ポインタの引数は A と X で渡す）
        ldx #>TITLE           ; 上位を X に
        jsr print               ; print を呼ぶ
        lda #$0D                ; putc('\r')
        jsr putc                ; 改行する
                                ; clear_meteors()：書き込みがすべて次のループで上書きされるので削除（-O2 から引き継ぐ）
        ldx #0                  ; for m in Mid { respawn(m) }：m を X に置ける（呼び出しがない）
@rs:    lda SEED                ; respawn 展開：rand 展開：rand_step(SEED)
        asl a                   ; rand_step 展開：左に1ビットずらす
        bcc @n1                 ; はみ出たビットが 0 なら XOR しない
        eor #$1D                ; 1 なら XOR を取る
@n1:    sta SEED                ; SEED = …
        sta rs_r                ; let r = …（2回使うのでしまう）
        and #7                  ; Mid::wrap(r)
        tay                     ; 表の添字にする
        lda SPAWN_X,y           ; SPAWN_X[…]
        sta M_X,x               ; METEORS[m].x = …
        stz M_Y,x               ; METEORS[m].y = 0
        lda rs_r                ; r を読み直す
        and #3                  ; r & 3
        sta M_WAIT,x            ; METEORS[m].wait = …
        inx                     ; 次の m へ
        cpx #8                  ; 8 に達したか
        bne @rs                 ; 達していなければ繰り返す
        cli                     ; irq_enable()
        php                     ; let mut last = ticks()（ticks 展開：atomic の始まり）
        sei                     ; 割り込みを止める
        lda TICKS               ; TICKS の下位
        ldx TICKS+1             ; TICKS の上位
        plp                     ; atomic の終わり
        sta mn_last             ; last の下位
        stx mn_last+1           ; last の上位

bench_loop:                     ; 'game: loop の先頭（wait_frame を展開）
        php                     ; ticks 展開：atomic の始まり
        sei                     ; 割り込みを止める
        lda TICKS               ; now の下位
        ldx TICKS+1             ; now の上位
        plp                     ; atomic の終わり
        cmp mn_last             ; now != last：下位を比べる
        bne wf_go               ; 違えば進む
        cpx mn_last+1           ; 上位を比べる
        bne wf_go               ; 違えば進む
        wai                     ; wait_interrupt()
        bra bench_loop          ; もう一度調べる
wf_go:
        sta mn_last             ; last = now：下位
        stx mn_last+1           ; 上位
bench_frame_begin:              ; ---- フレームの処理（read_key と move_player を展開） ----
        lda ACIA_STATUS         ; poll 展開：受信したか
        and #$08                ; 受信データありのビット
        beq bench_upd           ; None なら入力の処理を飛ばす（? と if let がつながった）
        lda ACIA_DATA           ; 受信した文字
        cmp #'A'                ; match c：'A'
        beq k_left              ; Key::Left → move_player の Left へ直接（定数の伝わり）
        cmp #'a'                ; 'a'
        beq k_left              ; Left へ
        cmp #'D'                ; 'D'
        beq k_right             ; Right へ
        cmp #'d'                ; 'd'
        beq k_right             ; Right へ
        cmp #'Q'                ; 'Q'
        beq k_quit              ; Quit → break 'game
        cmp #'q'                ; 'q'
        beq k_quit              ; Quit へ
        bra bench_upd           ; _ => None
k_quit: jmp game_over           ; break 'game（遠いので jmp）
k_left: lda PLAYER_X            ; PLAYER_X.saturating_sub(1)
        sec                     ; 引き算の準備
        sbc #1                  ; 1 を引く
        bcs @st                 ; 0 未満にならなければそのまま
        lda #0                  ; 0 で止める
@st:    sta PLAYER_X            ; 書く
        bra bench_upd           ; 更新へ
k_right:
        lda PLAYER_X            ; if PLAYER_X < 19
        cmp #19                 ; 19 と比べる
        bcs bench_upd           ; 19 以上なら何もしない
        inc PLAYER_X            ; PLAYER_X += 1
bench_upd:                      ; ---- update_meteors を展開 ----
        stz um_hit              ; let mut hit = false
        ldx #0                  ; for m in Mid：m を X に置ける（展開したので呼び出しがない）
upd_loop:
        lda M_WAIT,x            ; if METEORS[m].wait > 0
        beq upd_fall            ; 0 なら落下へ
        dec M_WAIT,x            ; wait -= 1（zp,x なので 6 サイクル）
        bra upd_next            ; continue
upd_fall:
        lda #3                  ; wait = 3
        sta M_WAIT,x            ; 書く
        inc M_Y,x               ; y += 1
        lda M_Y,x               ; if y == FIELD_H
        cmp #16                 ; 16 と比べる
        bne upd_chk             ; 違えば else if へ
        lda SEED                ; respawn(m) 展開：rand_step(SEED)
        asl a                   ; 左に1ビットずらす
        bcc @n2                 ; はみ出たビットが 0 なら XOR しない
        eor #$1D                ; XOR を取る
@n2:    sta SEED                ; SEED = …
        sta rs_r                ; let r = …
        and #7                  ; Mid::wrap(r)
        tay                     ; 表の添字にする
        lda SPAWN_X,y           ; SPAWN_X[…]
        sta M_X,x               ; METEORS[m].x = …
        stz M_Y,x               ; METEORS[m].y = 0
        lda rs_r                ; r を読み直す
        and #3                  ; r & 3
        sta M_WAIT,x            ; METEORS[m].wait = …
        sed                     ; SCORE += 1：10進モード
        clc                     ; 足し算の準備
        lda SCORE               ; 下位2桁
        adc #$01                ; 1 を足す
        sta SCORE               ; 書く
        lda SCORE+1             ; 上位2桁
        adc #$00                ; 繰り上がりを足す
        sta SCORE+1             ; 書く
        cld                     ; 10進モードを解除する
        bra upd_next            ; 次の m へ
upd_chk:
        cmp #15                 ; else if y == PLAYER_Y
        bne upd_next            ; 違えば次へ
        lda M_X,x               ; && x == PLAYER_X
        cmp PLAYER_X            ; 比べる
        bne upd_next            ; 違えば次へ
        lda #1                  ; hit = true
        sta um_hit              ; 書く
upd_next:
        inx                     ; 次の m へ
        cpx #8                  ; 8 に達したか
        bne upd_loop            ; 達していなければ繰り返す
        lda um_hit              ; if update_meteors()（戻り値の bool を直接調べる）
        beq bench_draw          ; false なら表示へ
        jmp game_over           ; true なら break 'game
bench_draw:                     ; ---- draw を展開 ----
        lda #$0D                ; putc('\r')
        jsr putc                ; 出す
        lda #<S_SCORE         ; 文字列のアドレスの下位を A に（ポインタの引数は A と X で渡す）
        ldx #>S_SCORE         ; 上位を X に
        jsr print               ; print を呼ぶ
        jsr print_score         ; print_score()
bench_frame_end:                ; ---- フレームの処理の終わり ----
        jmp bench_loop          ; 'game: loop の先頭へ
game_over:                      ; 'game: loop の後ろ
        lda #<S_OVER          ; 文字列のアドレスの下位を A に（ポインタの引数は A と X で渡す）
        ldx #>S_OVER          ; 上位を X に
        jsr print               ; print を呼ぶ
        jsr print_score         ; print_score()
        lda #$0D                ; putc('\r')
        jsr putc                ; 出す
        jmp MONITOR             ; main の終わり：exit = monitor

code_end:

; ============================================================================
; 定数
; ============================================================================
SPAWN_X:                        ; 出現位置の表
        .byte 3, 10, 17, 4, 11, 18, 5, 12
TITLE:                          ; p"METEOR DODGE"
        .byte 12, "METEOR DODGE"
S_SCORE:                        ; p"SCORE "
        .byte 6, "SCORE "
S_OVER:                         ; p"\rGAME OVER  SCORE "
        .byte 18, $0D, "GAME OVER  SCORE "
data_end:
vars_end:                       ; 通常の RAM の変数はない（すべてゼロページ）
