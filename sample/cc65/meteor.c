/*
 * meteor.c - METEOR DODGE の cc65 版（ax65 との性能比較用）
 *
 * ゲームの仕様は ax65 版（meteor/src/meteor.ax65）と同じで、端末への出力も同じになる。
 * ターゲットは v2（実機）：W65C02S 約 1.8425MHz、$0200 に読み込んで実行（HEXMON の JMP キー）、
 * UART（6551 互換）は $8400、GPIO（6522 互換）は $8000、HEXPAD は $8800。割り込みは使えない。
 * フレームの時間は GPIO のタイマー T2（ワンショット）で計る。終わったら rts でモニタへ戻る。
 *
 * cc65 で速いコードを得るための一般的な書き方に従っている：
 *   - 変数はグローバル（static）、型は unsigned char を基本にする
 *   - 構造体の配列ではなく、フィールドごとの配列にする
 *
 * 計測用の目印：MARK(n) は $7F00+n に stz で書き込む（4 サイクル。測定の側で差し引く）。
 * 実機で動かすときは -DNO_MARK を付けて作ると、目印の命令が入らない。
 */

#define ACIA_DATA    (*(volatile unsigned char *)0x8400)
#define ACIA_STATUS  (*(volatile unsigned char *)0x8401)
#define ACIA_CMD     (*(volatile unsigned char *)0x8402)
#define GPIO_T2C_L   (*(volatile unsigned char *)0x8008)
#define GPIO_T2C_H   (*(volatile unsigned char *)0x8009)
#define GPIO_IFR     (*(volatile unsigned char *)0x800D)
#define HP_KEY       (*(volatile unsigned char *)0x8800)

#ifdef NO_MARK
#define MARK_LOOP
#define MARK_BEGIN
#define MARK_UPD
#define MARK_DRAW
#define MARK_END
#else
#define MARK_LOOP    __asm__ ("stz $7F04")
#define MARK_BEGIN   __asm__ ("stz $7F00")
#define MARK_UPD     __asm__ ("stz $7F01")
#define MARK_DRAW    __asm__ ("stz $7F02")
#define MARK_END     __asm__ ("stz $7F03")
#endif

#define FIELD_W   20
#define FIELD_H   16
#define PLAYER_Y  (FIELD_H - 1)
#define NMET      8
#define FRAME_PERIOD 30706u           /* 1842500 / 60 - 2 */

#define KEY_LEFT  0
#define KEY_RIGHT 1
#define KEY_QUIT  2
#define KEY_NONE  0xFF

static const unsigned char spawn_x[NMET] = { 3, 10, 17, 4, 11, 18, 5, 12 };

static unsigned char m_x[NMET];
static unsigned char m_y[NMET];
static unsigned char m_wait[NMET];
static unsigned char player_x = FIELD_W / 2;
static unsigned char score_lo;        /* BCD 下位 2 桁 */
static unsigned char score_hi;        /* BCD 上位 2 桁 */
static unsigned char seed = 0x5A;


static void putc_acia(unsigned char c)
{
    while ((ACIA_STATUS & 0x10) == 0) {
    }
    ACIA_DATA = c;
}

static void print(const char *s)
{
    while (*s) {
        putc_acia(*s);
        ++s;
    }
}

static void print_score(void)
{
    putc_acia('0' + (score_hi >> 4));
    putc_acia('0' + (score_hi & 0x0F));
    putc_acia('0' + (score_lo >> 4));
    putc_acia('0' + (score_lo & 0x0F));
}

static void add_score(void)           /* BCD で 1 を足す（9999 の次は 0000） */
{
    ++score_lo;
    if ((score_lo & 0x0F) == 0x0A) {
        score_lo += 6;
        if (score_lo == 0xA0) {
            score_lo = 0;
            ++score_hi;
            if ((score_hi & 0x0F) == 0x0A) {
                score_hi += 6;
                if (score_hi == 0xA0) {
                    score_hi = 0;
                }
            }
        }
    }
}

static void frame_start(void)         /* T2 に 1/60 秒を設定して数え始める */
{
    GPIO_T2C_L = (unsigned char)(FRAME_PERIOD & 0xFF);
    GPIO_T2C_H = (unsigned char)(FRAME_PERIOD >> 8);
}

static void frame_wait(void)          /* T2 が 0 まで数えるのを待つ */
{
    while ((GPIO_IFR & 0x20) == 0) {
    }
}

static unsigned char rand8(void)      /* ガロア型 LFSR */
{
    unsigned char c = seed & 0x80;
    seed <<= 1;
    if (c) {
        seed ^= 0x1D;
    }
    return seed;
}

static unsigned char read_key(void)
{
    unsigned char k = HP_KEY;         /* 1 回だけ読む（読むとビット7 が消える） */
    if ((k & 0x80) == 0) {
        return KEY_NONE;
    }
    switch (k & 0x1F) {
    case 0x04: return KEY_LEFT;       /* 4 のキー */
    case 0x06: return KEY_RIGHT;      /* 6 のキー */
    case 0x12: return KEY_QUIT;       /* A/D のキー */
    }
    return KEY_NONE;
}

static void move_player(unsigned char k)
{
    if (k == KEY_LEFT) {
        if (player_x != 0) {
            --player_x;
        }
    } else if (k == KEY_RIGHT) {
        if (player_x < FIELD_W - 1) {
            ++player_x;
        }
    }
}

static void clear_meteors(void)
{
    unsigned char m;
    for (m = 0; m < NMET; ++m) {
        m_y[m] = 0;
        m_wait[m] = m;
    }
}

static void respawn(unsigned char m)
{
    unsigned char r = rand8();
    m_x[m] = spawn_x[r & 7];
    m_y[m] = 0;
    m_wait[m] = r & 3;
}

static unsigned char update_meteors(void)
{
    unsigned char m;
    unsigned char hit = 0;
    for (m = 0; m < NMET; ++m) {
        if (m_wait[m] != 0) {
            --m_wait[m];
            continue;
        }
        m_wait[m] = 3;
        ++m_y[m];
        if (m_y[m] == FIELD_H) {
            respawn(m);
            add_score();
        } else if (m_y[m] == PLAYER_Y && m_x[m] == player_x) {
            hit = 1;
        }
    }
    return hit;
}

static void draw(void)
{
    putc_acia('\r');
    print("SCORE ");
    print_score();
}

int main(void)
{
    unsigned char m;
    unsigned char k;

    ACIA_CMD = 0x0B;                  /* UART を割り込みなしで使う */

    print("METEOR DODGE");
    putc_acia('\r');

    clear_meteors();
    for (m = 0; m < NMET; ++m) {
        respawn(m);
    }

    frame_start();
    for (;;) {
        MARK_LOOP;
        frame_wait();
        MARK_BEGIN;
        frame_start();
        k = read_key();
        if (k != KEY_NONE) {
            if (k == KEY_QUIT) {
                break;
            }
            move_player(k);
        }
        MARK_UPD;
        if (update_meteors()) {
            break;
        }
        MARK_DRAW;
        draw();
        MARK_END;
    }

    print("\rGAME OVER  SCORE ");
    print_score();
    putc_acia('\r');
    return 0;
}
