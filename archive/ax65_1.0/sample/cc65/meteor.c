/*
 * meteor.c - METEOR DODGE の cc65 版（ax65 との性能比較用）
 *
 * ゲームの仕様は ax65 版（project/src/meteor.ax65）と同じで、端末への出力も同じになる。
 * ターゲットは v2 相当：W65C02S、1MHz、$0300 に読み込んで実行、ACIA は $8000、VIA は $6000、
 * IRQ はモニタの ROM から jmp ($0280) で来る、終わったら $FF1F（モニタ）へ戻る。
 *
 * cc65 で速いコードを得るための一般的な書き方に従っている：
 *   - 変数はグローバル（static）、型は unsigned char を基本にする
 *   - 構造体の配列ではなく、フィールドごとの配列にする
 *   - 割り込み処理は crt0.s にアセンブリで書く（cc65 では一般的な書き方）
 *
 * 計測用の目印：MARK(n) は $7000+n に stz で書き込む（4 サイクル。測定の側で差し引く）。
 */

#define ACIA_DATA    (*(volatile unsigned char *)0x8000)
#define ACIA_STATUS  (*(volatile unsigned char *)0x8001)
#define VIA_T1C_L    (*(volatile unsigned char *)0x6004)
#define VIA_T1C_H    (*(volatile unsigned char *)0x6005)
#define VIA_ACR      (*(volatile unsigned char *)0x600B)
#define VIA_IER      (*(volatile unsigned char *)0x600E)

#define MARK_LOOP    __asm__ ("stz $7004")
#define MARK_BEGIN   __asm__ ("stz $7000")
#define MARK_UPD     __asm__ ("stz $7001")
#define MARK_DRAW    __asm__ ("stz $7002")
#define MARK_END     __asm__ ("stz $7003")

#define FIELD_W   20
#define FIELD_H   16
#define PLAYER_Y  (FIELD_H - 1)
#define NMET      8
#define T1_PERIOD 16664u              /* 1000000 / 60 - 2 */

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

volatile unsigned int ticks;          /* 割り込み処理（crt0.s）が 1/60 秒ごとに増やす */
extern void wait_irq(void);           /* crt0.s：wai 命令で割り込みを待つ */

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

static unsigned int get_ticks(void)
{
    unsigned int t;
    __asm__ ("php");
    __asm__ ("sei");
    t = ticks;
    __asm__ ("plp");
    return t;
}

static unsigned int wait_frame(unsigned int last)
{
    unsigned int now;
    for (;;) {
        now = get_ticks();
        if (now != last) {
            return now;
        }
        wait_irq();                   /* wai（crt0.s。フレームを待つ間だけ呼ぶので、測定の区間には入らない） */
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
    unsigned char c;
    if ((ACIA_STATUS & 0x08) == 0) {
        return KEY_NONE;
    }
    c = ACIA_DATA;
    switch (c) {
    case 'A': case 'a': return KEY_LEFT;
    case 'D': case 'd': return KEY_RIGHT;
    case 'Q': case 'q': return KEY_QUIT;
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
    unsigned int last;

    VIA_ACR   = 0x40;                 /* タイマー1 を連続モードに */
    VIA_T1C_L = (unsigned char)(T1_PERIOD & 0xFF);
    VIA_T1C_H = (unsigned char)(T1_PERIOD >> 8);
    VIA_IER   = 0xC0;                 /* タイマー1 の割り込みを許可 */

    print("METEOR DODGE");
    putc_acia('\r');

    clear_meteors();
    for (m = 0; m < NMET; ++m) {
        respawn(m);
    }

    __asm__ ("cli");

    last = get_ticks();
    for (;;) {
        MARK_LOOP;
        last = wait_frame(last);
        MARK_BEGIN;
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
