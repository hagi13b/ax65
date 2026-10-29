; crt0.s - meteor の cc65 版の起動処理（ax65 との比較用。v2：HEXMON から JMP キーで呼ばれ、rts で戻る）
;
; ax65 版の起動処理（仕様書 1.1 15.1節、exit = rts）と同じことをする。
; 注意：cc65 の版によって、C のスタックポインタの名前が sp か c_sp かが違う。
;       アセンブル時に「sp が未定義」となる場合は、下の CSTK = sp を CSTK = c_sp に変える。

        .setcpu   "65C02"                ; 65C02 の命令（stz など）を使う
        .export   _init, _exit           ; 起動処理と終了処理の入口を公開する
        .export   __STARTUP__ : absolute = 1   ; ライブラリの crt0 の代わりにこの起動処理を使う
        .import   _main                  ; C の main
        .import   zerobss                ; BSS を 0 にするライブラリのルーチン
        .import   __MAIN_START__, __MAIN_SIZE__, __STACKSIZE__   ; リンカ設定の値
        .include  "zeropage.inc"         ; sp（または c_sp）などのゼロページの名前

CSTK    = sp                             ; C のスタックポインタ（c_sp の版なら c_sp に変える）

        .segment  "STARTUP"              ; 読み込み先（$0200）の先頭に置く
_init:  sei                              ; 割り込みを止める（このマシンでは割り込みを使わない）
        cld                              ; 10進モードを解除する
        tsx                              ; モニタから呼ばれたときのスタックポインタを
        stx     save_s                   ; 覚えておく（ハードウェアスタックはモニタのものをそのまま使う）
        lda     #<(__MAIN_START__ + __MAIN_SIZE__ + __STACKSIZE__)   ; C のスタックの先頭（下位）
        sta     CSTK                     ; 書く
        lda     #>(__MAIN_START__ + __MAIN_SIZE__ + __STACKSIZE__)   ; 上位
        sta     CSTK+1                   ; 書く
        jsr     zerobss                  ; 初期値のない変数（BSS）を 0 にする
        jsr     _main                    ; main を呼ぶ
_exit:  ldx     save_s                   ; exit() から来ても戻れるように、スタックポインタを戻す
        txs                              ; 設定する
        rts                              ; モニタへ戻る

        .segment  "DATA"                 ; 変数
save_s: .byte   0                        ; モニタから呼ばれたときのスタックポインタ
