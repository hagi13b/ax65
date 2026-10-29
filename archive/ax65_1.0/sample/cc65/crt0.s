; crt0.s - meteor の cc65 版の起動処理と割り込み処理（ax65 との比較用）
;
; ax65 版の起動処理（仕様書 15.1節）と同じことをする。
; 注意：cc65 の版によって、C のスタックポインタの名前が sp か c_sp かが違う。
;       アセンブル時に「sp が未定義」となる場合は、下の CSTK = sp を CSTK = c_sp に変える。

        .setcpu   "65C02"                ; 65C02 の命令（stz など）を使う
        .export   _init, _exit           ; 起動処理と終了処理の入口を公開する
        .export   _wait_irq              ; C から呼ぶ割り込み待ち（wait_irq）
        .export   __STARTUP__ : absolute = 1   ; ライブラリの crt0 の代わりにこの起動処理を使う
        .import   _main                  ; C の main
        .import   zerobss                ; BSS を 0 にするライブラリのルーチン
        .import   __MAIN_START__, __MAIN_SIZE__, __STACKSIZE__   ; リンカ設定の値
        .import   _ticks                 ; meteor.c の ticks（割り込み処理が増やす）
        .include  "zeropage.inc"         ; sp（または c_sp）などのゼロページの名前

CSTK    = sp                             ; C のスタックポインタ（c_sp の版なら c_sp に変える）
IFR     = $600D                          ; VIA 割り込みフラグ
T1C_L   = $6004                          ; VIA タイマー1 下位（読むと割り込みフラグが消える）
IRQ_VEC = $0280                          ; モニタの ROM が jmp ($0280) で飛んでくる場所
MONITOR = $FF1F                          ; 終わったら戻るモニタの入口

        .segment  "STARTUP"              ; 読み込み先（$0300）の先頭に置く
_init:  sei                              ; 割り込みを止める
        cld                              ; 10進モードを解除する
        ldx     #$FF                     ; ハードウェアスタックの初期値
        txs                              ; 設定する
        lda     #<(__MAIN_START__ + __MAIN_SIZE__ + __STACKSIZE__)   ; C のスタックの先頭（下位）
        sta     CSTK                     ; 書く
        lda     #>(__MAIN_START__ + __MAIN_SIZE__ + __STACKSIZE__)   ; 上位
        sta     CSTK+1                   ; 書く
        jsr     zerobss                  ; 初期値のない変数（BSS）を 0 にする
        lda     #<irq                    ; 割り込み処理のアドレスの下位
        sta     IRQ_VEC                  ; RAM 上の IRQ ベクタに書く
        lda     #>irq                    ; 上位
        sta     IRQ_VEC+1                ; 書く
        jsr     _main                    ; main を呼ぶ
_exit:  jmp     MONITOR                  ; モニタへ戻る

_wait_irq:                               ; void wait_irq(void)
        .byte   $CB                      ; wai 命令（割り込みが来るまで CPU を止める。ca65 の 65C02 では名前がないので数値で書く）
        rts                              ; 戻る

irq:    bit     IFR                      ; タイマー1（ビット6）の割り込みかを V フラグで調べる
        bvc     @done                    ; 違えば終わる
        bit     T1C_L                    ; 読み捨てて割り込みフラグを消す（A は変わらない）
        inc     _ticks                   ; ticks の下位を 1 増やす
        bne     @done                    ; 0 に戻らなければ終わり
        inc     _ticks+1                 ; 上位に繰り上げる
@done:  rti                              ; 割り込みから戻る
