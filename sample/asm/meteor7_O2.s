; ============================================================================
; meteor7_O2.s — meteor7.ax65（7セグメント版）を -O2（既定）でコンパイルした想定出力
;
; AX65 仕様書 1.1 の規則に従って手で生成した「想定出力」（リファレンスコンパイラの出力ではない）。
; ソース：meteor7/src/meteor7.ax65、ターゲット：v2（meteor7/targets/v2.target）
; 使うモジュール：hw/hexpad.ax65（poll_key、show、show_bcd）、hw/hexmon.ax65（delay_ms）
; 打ち込み用のダンプは meteor7_O2.woz（HEXMON で 0200 から打ち込み、0200 で JMP）。
;
; -O2 で行ったこと（19.3節）：
;   ・ゼロページへの自動配置（$20〜$FF の区画）
;   ・展開は 1か所からしか呼ばれない関数（main、step、move_player、rand、rand_step、show_bcd）と、
;     #[inline] を付けた hexpad::show だけ。2か所から呼ばれる draw と poll_key は関数のまま
;   ・ループの逆順化はしない（draw は I/O に書き、step は途中で return するので、順番に依存する。6.4節）
;   ・ループの配置：draw のループの分岐は $0300 のページ境界をまたぐ（1 列ごとに 1 サイクル）。並び順の入れ替えでは
;     そろえられず、-O2 は空きのバイトを入れないので、そのままにした（19.3節。-Os なら空きを入れてそろえる）
;   ・show_bcd の for d in v.digits() は 4 回で、p と Digit::check(p) がコンパイル時に決まるので、展開して検査を消した
;   ・delay_ms は extern fn で、A とフラグしか壊さない（clobbers(A, flags)）ので、X と Y は呼び出しをまたいで使える
; ============================================================================

DELAY_MS = $FF09                ; extern fn delay_ms（HEXMON の API）
HP_KEY   = $8800                ; hexpad の HP_KEY（io(in)。読むとビット7 が消える）
HP_DIGIT = $8808                ; hexpad の HP_DIGIT（io(out)。6 桁、0 番が右端）

; ---- ゼロページ（-O の自動配置。$00〜$1F はモニタが使うので $20 から） --------
METEOR   = $20                  ; static METEOR: [bool; Digit]（6 バイト。0 か 1）
PLAYER   = $26                  ; static PLAYER: Digit
SCORE    = $27                  ; static SCORE: bcd16（2バイト）
SEED     = $29                  ; static SEED
SPEED    = $2A                  ; static SPEED
mn_n     = $2B                  ; main の for _ in 0..SPEED の残りの回数
st_r     = $2C                  ; step（展開済み）の let r

        .org $0200              ; load = $0200

; ============================================================================
; 起動処理（main を展開している）
; ============================================================================
start:                          ; HEXMON で 0200 を表示して JMP キーで実行される入口
        sei                     ; 割り込みを止める（irq = none なので、この後も許可しない）
        cld                     ; 10進モードを解除する（exit = rts なので、スタックポインタは設定しない）
        lda #2                  ; PLAYER = 2
        sta PLAYER              ; 書く
        lda #$5A                ; SEED = $5A
        sta SEED                ; 書く
        lda #60                 ; SPEED = 60
        sta SPEED               ; 書く
        stz SCORE               ; 初期値が 0 の変数は stz で埋める（SCORE 下位）
        stz SCORE+1             ; SCORE 上位
        ldx #5                  ; METEOR の 6 バイトを false（0）にする
@zero:  stz METEOR,x            ; 0 を書く
        dex                     ; 次のバイトへ
        bpl @zero               ; 0 番目まで繰り返す
        ; ---- ここから main（展開済み） ----
game:                           ; 'game: loop の先頭
        jsr draw                ; draw()
        lda SPEED               ; for _ in 0..SPEED：回数を読む
        beq do_step             ; 0 回なら繰り返さない
        sta mn_n                ; 残りの回数にする
poll:                           ; for の中身
        lda #10                 ; delay_ms(10)：引数は A
        jsr DELAY_MS            ; HEXMON の DELAY_MS を呼ぶ（A とフラグだけが壊れる）
        jsr poll_key            ; if let Some(k) = hexpad::poll_key()
        bcc next                ; None なら次の回へ
        cmp #$12                ; if k == $12（A/D のキー）
        beq over                ; break 'game
        cmp #$04                ; move_player 展開：match k：$04（4 のキー）
        bne @right              ; 違えば $06 を調べる
        ldx PLAYER              ; PLAYER as u8 + 1
        inx                     ; 1 を足す
        bra @check              ; Digit::check へ
@right: cmp #$06                ; $06（6 のキー）
        bne redraw              ; _ => {}
        ldx PLAYER              ; PLAYER as u8 - 1
        dex                     ; 1 を引く（0 なら 255 になる）
@check: cpx #6                  ; Digit::check：6 以上なら None
        bcs redraw              ; None なら動かない
        stx PLAYER              ; PLAYER = p
redraw: jsr draw                ; draw()
next:   dec mn_n                ; 残りの回数を 1 減らす
        bne poll                ; 残っていれば繰り返す
do_step:                        ; ---- step を展開 ----
        ldx #0                  ; for l in Digit（途中で return するので、前から順に）
@fall:  lda METEOR,x            ; if METEOR[l]
        beq @nx                 ; false なら次の列へ
        cpx PLAYER              ; if l == PLAYER
        beq over                ; return true → if step() { break 'game }
        stz METEOR,x            ; METEOR[l] = false
        sed                     ; SCORE += 1：10進モード
        clc                     ; 足し算の準備
        lda SCORE               ; 下位2桁
        adc #$01                ; 1 を足す
        sta SCORE               ; 書く
        lda SCORE+1             ; 上位2桁
        adc #$00                ; 繰り上がりを足す
        sta SCORE+1             ; 書く
        cld                     ; 10進モードを解除する
        lda SPEED               ; if SPEED > MIN_SPEED
        cmp #21                 ; SPEED >= 21 と同じ
        bcc @nx                 ; 20 以下なら減らさない
        dec SPEED               ; SPEED -= 1
@nx:    inx                     ; 次の列へ
        cpx #6                  ; 6 列調べたか
        bne @fall               ; まだなら繰り返す
        lda SEED                ; let r = rand()：rand 展開：rand_step(SEED)
        asl a                   ; 左に1ビットずらす
        bcc @n                  ; はみ出たビットが 0 なら XOR しない
        eor #$1D                ; XOR を取る
@n:     sta SEED                ; SEED = …
        sta st_r                ; r（あとでもう一度使う）
        and #7                  ; r & 7
        cmp #6                  ; Digit::check：6 以上なら None
        bcs @two                ; None なら出さない
        tax                     ; 列の番号を X に
        lda #1                  ; true
        sta METEOR,x            ; METEOR[l] = true
@two:   lda st_r                ; if r & $C0 == 0
        and #$C0                ; ビット7 と 6 を取り出す
        bne @done               ; 0 でなければ 2 個目は出さない
        lda st_r                ; (r >> 3) & 7
        lsr a                   ; 右に1ビット（1回目）
        lsr a                   ; （2回目）
        lsr a                   ; （3回目）
        and #7                  ; 下位3ビット
        cmp #6                  ; Digit::check：6 以上なら None
        bcs @done               ; None なら出さない
        tax                     ; 列の番号を X に
        lda #1                  ; true
        sta METEOR,x            ; METEOR[l] = true
@done:  jmp game                ; false を返す → 'game: loop の先頭へ（遠いので jmp）
over:                           ; 'game: loop の後ろ
        lda #250                ; delay_ms(250)
        jsr DELAY_MS            ; 待つ
        lda #250                ; delay_ms(250)
        jsr DELAY_MS            ; 待つ
        lda #16                 ; hexpad::show(5, MINUS) 展開
        sta HP_DIGIT+5          ; 左端
        sta HP_DIGIT+4          ; hexpad::show(4, MINUS) 展開（A に MINUS が残っている）
        lda SCORE+1             ; show_bcd 展開：千の位（上位2桁の上の4ビット）
        lsr a                   ; 上位4ビットを下へ（1回目）
        lsr a                   ; （2回目）
        lsr a                   ; （3回目）
        lsr a                   ; （4回目）
        sta HP_DIGIT+3          ; 右から 4 桁目
        lda SCORE+1             ; 百の位
        and #$0F                ; 下位4ビット
        sta HP_DIGIT+2          ; 右から 3 桁目
        lda SCORE               ; 十の位
        lsr a                   ; 上位4ビットを下へ（1回目）
        lsr a                   ; （2回目）
        lsr a                   ; （3回目）
        lsr a                   ; （4回目）
        sta HP_DIGIT+1          ; 右から 2 桁目
        lda SCORE               ; 一の位
        and #$0F                ; 下位4ビット
        sta HP_DIGIT            ; 右端
@wait:  jsr poll_key            ; loop { if let Some(_) = hexpad::poll_key() { break; } }
        bcc @wait               ; None なら待つ
        rts                     ; main の終わり：exit = rts でモニタへ戻る

; ============================================================================
; 残した関数（2か所から呼ばれる）
; ============================================================================
draw:                           ; fn draw()
        ldx #0                  ; for l in Digit：l = 0（右端）から
@l:     lda METEOR,x            ; if METEOR[l]
        beq @nm                 ; false なら else if へ
        cpx PLAYER              ; if l == PLAYER
        beq @both               ; 同じなら SMALL_O
        lda #16                 ; MINUS（-）
        bra @st                 ; 書きに行く
@both:  lda #0                  ; SMALL_O（o）
        bra @st                 ; 書きに行く
@nm:    cpx PLAYER              ; else if l == PLAYER
        bne @blank              ; 違えば BLANK
        lda #31                 ; SMALL_U（u）
        bra @st                 ; 書きに行く
@blank: lda #20                 ; BLANK（空白）
@st:    sta HP_DIGIT,x          ; hexpad::show(l, code) 展開
        inx                     ; 次の列へ
        cpx #6                  ; 6 列描いたか
        bne @l                  ; まだなら繰り返す
        rts                     ; 戻る

poll_key:                       ; hexpad::poll_key() -> Option<u8>（有無はキャリー、値は A）
        lda HP_KEY              ; let k = HP_KEY（1回だけ読む）
        bpl @none               ; k & $80 == 0 なら None（ビット7 は N フラグに入る）
        and #$1F                ; Some(k & $1F)
        sec                     ; Some（キャリー 1）
        rts                     ; 戻る
@none:  clc                     ; None（キャリー 0）
        rts                     ; 戻る
code_end:                       ; コードの終わり
data_end:                       ; 定数はない
vars_end:                       ; 通常の RAM の変数はない（すべてゼロページ）
