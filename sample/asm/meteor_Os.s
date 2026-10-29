; ============================================================================
; meteor_Os.s — meteor.ax65 を -Os（速さ優先）でコンパイルした想定出力
;
; AX65 仕様書 1.1 の規則に従って手で生成した「想定出力」（リファレンスコンパイラの出力ではない）。
; ソース：meteor/src/meteor.ax65、ターゲット：v2（meteor/targets/v2.target）
; 使うモジュール：hw/acia_console.ax65（putc）、hw/hexpad.ax65（poll_key）、hw/frame_timer.ax65（start、wait）
;
; どの段階でも行う規則（仕様書 1.1 19.5節）：
;   ・初期値が 0 の変数は stz で初期化し、元データを持たない（4.6節）
;   ・末尾呼び出しは jmp（7.7節）
;   ・ポインタの引数は A（下位）と X（上位）で渡す（15.2節）
;   ・構造体の配列はフィールドごとに分ける（3.8節）
;   ・comptime if は v2 用に解決済み：screen がないので draw は else 側だけ（13.6節）
;   ・v2 は exit = rts、irq = none：起動処理はスタックポインタを設定せず、割り込みベクタも書かない（16.4節）
; bench_ で始まるラベルは計測用の目印で、生成されるコードには影響しない。
;
; -Os で行ったこと（19.4節）：-O2 のすべて（共通部分のくくり出しを除く）に加えて
;   ・2か所から呼ばれる小さな関数 respawn と frame_timer::start も、ループの中で呼ばれているので展開した
;     （#[cold] を付けていないため。7.7節）
;   ・for で直接回す SCORE.digits() を展開した（5.5節）
;   ・更新のループがページ境界をまたいでいないことを確かめた（並べ替えは不要だった）
; ============================================================================

T2C_L   = $8008                 ; frame_timer の T2C_L（タイマー T2 の下位。読むとフラグが消える。io）
T2C_H   = $8009                 ; frame_timer の T2C_H（T2 の上位。書くと数え始める。io(out)）
IFR     = $800D                 ; frame_timer の IFR（割り込みフラグ。io(in)、型は flags Ifr）
ACIA_DATA   = $8400             ; acia_console の DATA（io）
ACIA_STATUS = $8401             ; acia_console の STATUS（io(in)）
ACIA_CMD    = $8402             ; acia_console の CMD（io(out)）
HP_KEY  = $8800                 ; hexpad の HP_KEY（io(in)。読むとビット7 が消える）

; ---- ゼロページ（-O の自動配置。$00〜$1F はモニタが使うので $20 から） --------
pr_s      = $20                 ; print の引数 s（ポインタ、2バイト。print の入口で A と X から書く）
rs_r      = $22                 ; respawn の r
um_hit    = $23                 ; update_meteors（展開済み）の hit
putc_c    = $24                 ; putc の引数 c
PLAYER_X  = $25                 ; static PLAYER_X
SCORE     = $26                 ; static SCORE（2バイト）
SEED      = $28                 ; static SEED
M_X       = $29                 ; METEORS[*].x（8個）
M_Y       = $31                 ; METEORS[*].y（8個）
M_WAIT    = $39                 ; METEORS[*].wait（8個）

        .org $0200              ; load = $0200

; ============================================================================
; 起動処理（main を展開している）
; ============================================================================
start:                          ; HEXMON で 0200 を表示して JMP キーで実行される入口
        sei                     ; 割り込みを止める（irq = none なので、この後も許可しない）
        cld                     ; 10進モードを解除する（exit = rts なので、スタックポインタは設定しない）
        lda #10                 ; 初期値が 0 でない変数だけ即値で書く（PLAYER_X = 10）
        sta PLAYER_X            ; 書く
        lda #$5A                ; SEED = $5A
        sta SEED                ; 書く
        stz SCORE               ; 初期値が 0 の変数は stz で埋める（SCORE 下位）
        stz SCORE+1             ; SCORE 上位
        ldx #23                 ; METEORS の 24 バイト（x、y、wait）をまとめて 0 にする
@zero:  stz M_X,x               ; 0 を書く
        dex                     ; 次のバイトへ
        bpl @zero               ; 0 番目まで繰り返す
        lda #$0B                ; init（acia_console）：CMD = $0B
        sta ACIA_CMD            ; UART を割り込みなしで使う設定にする
        ; ---- ここから main（展開済み） ----
        lda #<TITLE             ; 文字列のアドレスの下位を A に
        ldx #>TITLE             ; 上位を X に
        jsr print               ; print を呼ぶ
        lda #$0D                ; putc('\r')
        jsr putc                ; 改行する
                                ; clear_meteors()：書き込みがすべて次のループで上書きされるので削除
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
        lda #$F2                ; frame_timer::start() 展開：T2C_L = $F2（PERIOD = 30706 = $77F2）
        sta T2C_L               ; 下位を先に書く
        lda #$77                ; T2C_H = $77
        sta T2C_H               ; 上位を書くと数え始める

bench_loop:                     ; 'game: loop の先頭（frame_timer::wait を展開）
        lda #$20                ; IFR.contains(Ifr::T2)：T2 のビット（ビット5）を A に
@wait:  bit IFR                 ; A と IFR の AND で Z フラグが決まる（A は変わらない）
        beq @wait               ; T2 のビットが 0 なら待つ
        bit T2C_L               ; let _ = T2C_L：読み捨てて T2 のフラグを消す（A は変わらない）
bench_frame_begin:              ; ---- フレームの処理 ----
        lda #$F2                ; frame_timer::start() 展開：T2C_L = $F2
        sta T2C_L               ; 下位を先に書く
        lda #$77                ; T2C_H = $77
        sta T2C_H               ; 上位を書くと数え始める
        lda HP_KEY              ; read_key 展開：hexpad::poll_key 展開：let k = HP_KEY（1回だけ読む）
        bpl bench_upd           ; k & $80 == 0 なら None（ビット7 は N フラグに入る）。入力の処理を飛ばす
        and #$1F                ; Some(k & $1F)：キーコード
        cmp #$04                ; match k：$04（4 のキー）
        beq k_left              ; Key::Left → move_player の Left へ直接（定数の伝わり）
        cmp #$06                ; $06（6 のキー）
        beq k_right             ; Key::Right → Right へ
        cmp #$12                ; $12（A/D のキー）
        bne bench_upd           ; _ => None
k_quit: jmp game_over           ; Key::Quit → break 'game（遠いので jmp）
k_left: lda PLAYER_X            ; PLAYER_X.saturating_sub(1)
        sec                     ; 引き算の準備
        sbc #1                  ; 1 を引く
        bcs @st                 ; 0 未満にならなければそのまま
        lda #0                  ; 0 で止める
@st:    sta PLAYER_X            ; 書く
        bra bench_upd           ; 更新へ
k_right:                        ; Key::Right
        lda PLAYER_X            ; if PLAYER_X < 19
        cmp #19                 ; 19 と比べる
        bcs bench_upd           ; 19 以上なら何もしない
        inc PLAYER_X            ; PLAYER_X += 1
bench_upd:                      ; ---- update_meteors を展開 ----
        stz um_hit              ; let hit = false
        ldx #0                  ; for m in Mid：m を X に置ける（展開したので呼び出しがない）
upd_loop:                       ; ループの先頭
        lda M_WAIT,x            ; if METEORS[m].wait > 0
        beq upd_fall            ; 0 なら落下へ
        dec M_WAIT,x            ; wait -= 1（zp,x なので 6 サイクル）
        bra upd_next            ; continue
upd_fall:                       ; 落下の処理
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
upd_chk:                        ; else if の判定
        cmp #15                 ; else if y == PLAYER_Y
        bne upd_next            ; 違えば次へ
        lda M_X,x               ; && x == PLAYER_X
        cmp PLAYER_X            ; 比べる
        bne upd_next            ; 違えば次へ
        lda #1                  ; hit = true
        sta um_hit              ; 書く
upd_next:                       ; 次の隕石へ
        inx                     ; 次の m へ
        cpx #8                  ; 8 に達したか
        bne upd_loop            ; 達していなければ繰り返す
        lda um_hit              ; if update_meteors()（戻り値の bool を直接調べる）
        beq bench_draw          ; false なら表示へ
        jmp game_over           ; true なら break 'game
bench_draw:                     ; ---- draw を展開 ----
        lda #$0D                ; putc('\r')
        jsr putc                ; 出す
        lda #<S_SCORE           ; 文字列のアドレスの下位を A に
        ldx #>S_SCORE           ; 上位を X に
        jsr print               ; print を呼ぶ
        jsr print_score         ; print_score()
bench_frame_end:                ; ---- フレームの処理の終わり ----
        jmp bench_loop          ; 'game: loop の先頭へ
game_over:                      ; 'game: loop の後ろ
        lda #<S_OVER            ; 文字列のアドレスの下位を A に
        ldx #>S_OVER            ; 上位を X に
        jsr print               ; print を呼ぶ
        jsr print_score         ; print_score()
        lda #$0D                ; putc('\r')
        jmp putc                ; 出して、putc の rts でモニタへ戻る（main の終わり＝exit = rts。末尾呼び出し）

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

print:                          ; s のアドレスは A（下位）と X（上位）で届く
        sta pr_s                ; ゼロページのポインタに書く（呼び出し先で1回だけ）
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

print_score:                    ; for d in SCORE.digits() を結果領域なしで展開
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
code_end:                       ; コードの終わり

; ============================================================================
; 定数
; ============================================================================
SPAWN_X:                        ; 出現位置の表
        .byte 3, 10, 17, 4, 11, 18, 5, 12 ; 8 個の出現位置（spawn_x(0)〜spawn_x(7)）
TITLE:                          ; p"METEOR DODGE"
        .byte 12, "METEOR DODGE" ; 先頭の 1 バイトが長さ、続いて文字の並び
S_SCORE:                        ; p"SCORE "
        .byte 6, "SCORE "       ; 先頭の 1 バイトが長さ、続いて文字の並び
S_OVER:                         ; p"\rGAME OVER  SCORE "
        .byte 18, $0D, "GAME OVER  SCORE " ; 先頭の 1 バイトが長さ、続いて文字の並び
data_end:                       ; 定数の終わり
vars_end:                       ; 通常の RAM の変数はない（すべてゼロページ）
