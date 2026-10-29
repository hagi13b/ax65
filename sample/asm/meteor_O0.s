; ============================================================================
; meteor_O0.s — meteor.ax65 を -O0（最適化なし）でコンパイルした想定出力
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
;   ・使われない関数は除く（acia_console の poll と getc。8.6節）
;   ・comptime if は v2 用に解決済み：screen がないので draw は else 側だけ（13.6節）
;   ・v2 は exit = rts、irq = none：起動処理はスタックポインタを設定せず、割り込みベクタも書かない（16.4節）
; bench_ で始まるラベルは計測用の目印で、生成されるコードには影響しない。
;
; -O0 の特徴：
;   ・ゼロページに置くのはポインタ（print の引数）だけ。変数は通常の RAM（4.7節）
;   ・ローカル変数は固定アドレスに重ねて置く（4.8節）
;   ・関数は展開しない。呼び出し先が壊すレジスタは解析して、必要な分だけ退避する（15.4節）
; ============================================================================

T2C_L   = $8008                 ; frame_timer の T2C_L（タイマー T2 の下位。読むとフラグが消える。io）
T2C_H   = $8009                 ; frame_timer の T2C_H（T2 の上位。書くと数え始める。io(out)）
IFR     = $800D                 ; frame_timer の IFR（割り込みフラグ。io(in)、型は flags Ifr）
ACIA_DATA   = $8400             ; acia_console の DATA（io）
ACIA_STATUS = $8401             ; acia_console の STATUS（io(in)）
ACIA_CMD    = $8402             ; acia_console の CMD（io(out)）
HP_KEY  = $8800                 ; hexpad の HP_KEY（io(in)。読むとビット7 が消える）

; ---- ゼロページ（ポインタだけが自動で置かれる。$00〜$1F はモニタが使うので $20 から）
pr_s    = $20                   ; print の引数 s（str はポインタで渡す。2バイト）

; ---- ローカル変数の重ね合わせ（4.8節）：同時に使われない関数同士で同じ番地を使う
mn_m      = LOCALS+0            ; main の for m in Mid のカウンタ（respawn が X/Y を壊すのでメモリに置く）
pk_k      = LOCALS+1            ; hexpad::poll_key の let k（main の後ろに置く）
um_hit    = LOCALS+1            ; update_meteors の let hit（poll_key と同時には使われない）
um_m      = LOCALS+2            ; update_meteors の for m in Mid のカウンタ
rs_r      = LOCALS+3            ; respawn の let r（update_meteors の後ろに置く）
putc_c    = LOCALS+1            ; putc の引数 c（後で使うのでしまう）
bd_v      = LOCALS+1            ; bcd16_digits の引数（2バイト）

        .org $0200              ; load = $0200

; ============================================================================
; 起動処理（コンパイラが自動で作る）
; ============================================================================
start:                          ; HEXMON で 0200 を表示して JMP キーで実行される入口
        sei                     ; 割り込みを止める（irq = none なので、この後も許可しない）
        cld                     ; 10進モードを解除しておく（exit = rts なので、スタックポインタは設定しない）
        lda #10                 ; 初期値が 0 でない変数だけ即値で書く：PLAYER_X = 10
        sta PLAYER_X            ; 書く
        lda #$5A                ; SEED = $5A
        sta SEED                ; 書く
        stz SCORE               ; 初期値が 0 の変数は stz で 0 にする（SCORE 下位）
        stz SCORE+1             ; SCORE 上位
        ldx #23                 ; METEORS の 24 バイト（x、y、wait）をまとめて 0 にする
@zero:  stz M_X,x               ; 0 を書く
        dex                     ; 次のバイトへ
        bpl @zero               ; 0 番目まで繰り返す
        ; ---- acia_console.ax65 の init ブロック（ここに展開される） ----
        lda #$0B                ; CMD = $0B
        sta ACIA_CMD            ; UART を割り込みなしで使う設定にする
        jmp main                ; main を呼ぶ。main の rts でそのままモニタへ戻る（exit = rts。末尾呼び出し）

; ============================================================================
; hw::frame_timer
; ============================================================================
ft_start:                       ; pub fn start()
        lda #$F2                ; T2C_L = (PERIOD & $FF) as u8（PERIOD = 30706 = $77F2）
        sta T2C_L               ; 下位を先に書く（I/O の順序はソースどおり）
        lda #$77                ; T2C_H = (PERIOD >> 8) as u8
        sta T2C_H               ; 上位を書くと数え始める
        rts                     ; 呼び出し元に戻る

ft_wait:                        ; pub fn wait()
@loop:  lda IFR                 ; while !IFR.contains(Ifr::T2)：IFR を読む
        and #$20                ; ビット5（T2）を取り出す
        beq @loop               ; 0 なら繰り返す
        bit T2C_L               ; let _ = T2C_L：読み捨てて T2 のフラグを消す（A は変わらない）
        rts                     ; 呼び出し元に戻る

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
; hw::acia_console の実装（impl console。使われる putc だけが残る）
; ============================================================================
putc:                           ; fn putc(c: char)：c は A で届く
        sta putc_c              ; c は待ちループの後で使うのでしまう
@wait:  lda ACIA_STATUS         ; 送信の準備ができたか調べる
        and #$10                ; ビット4（送信バッファが空）を取り出す
        beq @wait               ; まだなら待つ
        lda putc_c              ; しまっておいた c を読む
        sta ACIA_DATA           ; 1文字送る
        rts                     ; 呼び出し元に戻る（壊すのは A とフラグだけ）

; ============================================================================
; hw::hexpad
; ============================================================================
hp_poll_key:                    ; pub fn poll_key() -> Option<u8>（有無はキャリー、値は A）
        lda HP_KEY              ; let k = HP_KEY（1回だけ読む。読むとビット7 が消える）
        sta pk_k                ; k は後で使うのでしまう
        and #$80                ; if k & $80 == 0
        bne @some               ; 0 でなければ Some へ
        clc                     ; return None（キャリー 0）
        rts                     ; 呼び出し元に戻る
@some:  lda pk_k                ; k を読み直す
        and #$1F                ; k & $1F
        sec                     ; Some（キャリー 1）
        rts                     ; 値は A に入っている

; ============================================================================
; fn read_key() -> Option<Key>（Key：Left=0、Right=1、Quit=2）
; ============================================================================
read_key:                       ; 戻り値：キャリー＝有無、A＝Key
        jsr hp_poll_key         ; let k = hexpad::poll_key()?
        bcc @ret                ; ? 演算子：None ならキャリー 0 のまま抜ける
        cmp #$04                ; match k：$04（4 のキー）か
        beq @left               ; Left へ
        cmp #$06                ; $06（6 のキー）か
        beq @right              ; Right へ
        cmp #$12                ; $12（A/D のキー）か
        beq @quit               ; Quit へ
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
        stz um_hit              ; let hit = false
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
print:                          ; s のアドレスは A（下位）と X（上位）で届く（15.2節）
        sta pr_s                ; 入口でゼロページのポインタに書く
        stx pr_s+1              ; 上位も書く
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
        lda #<S_SCORE           ; 文字列のアドレスの下位を A に（ポインタの引数は A と X で渡す）
        ldx #>S_SCORE           ; 上位を X に
        jsr print               ; print を呼ぶ
        jmp print_score         ; print_score()（末尾呼び出しなので jmp）

; ============================================================================
; fn main()
; ============================================================================
main:                           ; プログラム本体
        lda #<TITLE             ; 文字列のアドレスの下位を A に（ポインタの引数は A と X で渡す）
        ldx #>TITLE             ; 上位を X に
        jsr print               ; print を呼ぶ
        lda #$0D                ; putc('\r')
        jsr putc                ; 改行する
        jsr clear_meteors       ; clear_meteors()
        stz mn_m                ; for m in Mid：m = 0 から（respawn が X/Y を壊すのでメモリ）
main_rs:                        ; ループの先頭
        lda mn_m                ; respawn(m)：m を A に入れる
        jsr respawn             ; respawn を呼ぶ
        inc mn_m                ; 次の m へ
        lda mn_m                ; m を読む
        cmp #8                  ; 8 に達したか
        bne main_rs             ; 達していなければ繰り返す
        jsr ft_start            ; frame_timer::start()
bench_loop:                     ; 'game: loop の先頭
        jsr ft_wait             ; frame_timer::wait()：前のフレームの始まりから 1/60 秒たつまで待つ
bench_frame_begin:              ; ---- ここからフレームの処理 ----
        jsr ft_start            ; frame_timer::start()：次のフレームの時間を計り始める
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
        lda #<S_OVER            ; 文字列のアドレスの下位を A に（ポインタの引数は A と X で渡す）
        ldx #>S_OVER            ; 上位を X に
        jsr print               ; print を呼ぶ
        jsr print_score         ; print_score()
        lda #$0D                ; putc('\r')
        jmp putc                ; 末尾呼び出しなので jmp（putc の rts でモニタへ戻る）
code_end:                       ; コードの終わり

; ============================================================================
; 定数（const）
; ============================================================================
SPAWN_X:                        ; const SPAWN_X: [u8; Mid]（comptime fn spawn_x で計算済み）
        .byte 3, 10, 17, 4, 11, 18, 5, 12 ; 8 個の出現位置（spawn_x(0)〜spawn_x(7)）
TITLE:                          ; const TITLE: str = p"METEOR DODGE"
        .byte 12, "METEOR DODGE" ; 先頭の 1 バイトが長さ、続いて文字の並び
S_SCORE:                        ; p"SCORE "
        .byte 6, "SCORE "       ; 先頭の 1 バイトが長さ、続いて文字の並び
S_OVER:                         ; p"\rGAME OVER  SCORE "
        .byte 18, $0D, "GAME OVER  SCORE " ; 先頭の 1 バイトが長さ、続いて文字の並び
data_end:                       ; 定数の終わり

; ============================================================================
; 変数（RAM、コードの後ろ）
; ============================================================================
PLAYER_X: .res 1                ; static PLAYER_X: u8
SCORE:    .res 2                ; static SCORE: bcd16
SEED:     .res 1                ; static SEED: u8
M_X:      .res 8                ; METEORS[*].x（フィールドごとの配列）
M_Y:      .res 8                ; METEORS[*].y
M_WAIT:   .res 8                ; METEORS[*].wait
LOCALS:   .res 4                ; ローカル変数の重ね合わせ領域（最大 4 バイト）
DIGITS:   .res 4                ; bcd16_digits の結果領域
vars_end:                       ; 変数の終わり
