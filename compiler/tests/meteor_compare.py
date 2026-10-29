#!/usr/bin/env python3
"""ax65 でコンパイルした meteor と、手書きの想定出力（asm/meteor_O0.s を tools/asm65.py で
組み立てたもの）を、sample/tools のシミュレータで動かし、端末（UART）への出力を比べる。

1.1 の sample/tools（実機 v2 用に書き直したもの）を使う。ここで使うのは次のものだけ：
  asm65.Asm().run(テキスト)       → .out（{番地: バイト}）と .sym（{名前: 番地}）を持つ結果
  sim65c02.Machine                 → .mem、.PC、.S、.cyc、.step()、受信（.rx か .feed()）、送信（.out）
  （あれば）sim65c02.SENTINEL      → 起動処理の rts が戻る番地（なければ $FFF0）
  （あれば）sim65c02.MONITOR       → ここへ飛んだら終わり（exit = monitor のプログラム用）

v2 は exit = rts なので、モニタと同じように、戻り番地（SENTINEL）を積んでから start を実行し、
rts で SENTINEL に戻ったら終わりとする。割り込みがない（irq = none）ので、フレームの区切りは使わない。
キーは「それまでの出力のバイト数」を合図に送る（出力が同じなら、どのビルドでも同じ時点で届く）。

使い方：python3 meteor_compare.py --tools <tools> --ref <meteor_O0.s> [--keys 300:a,600:d] [--quit 4000] <出力フォルダ>...
"""
import argparse, os, re, sys

DEFAULT_KEYS = "200:a,400:a,700:d,1000:d,1300:a,1600:d"
DEFAULT_QUIT = 3000          # この出力のバイト数で 'q' を送る
MAX_CYCLES = 200_000_000


def load_sym(path):
    sym = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"^\s*([A-Za-z_][\w@~:.]*)\s*=\s*\$([0-9A-Fa-f]+)", line)
            if m:
                sym[m.group(1)] = int(m.group(2), 16)
    return sym


def parse_keys(text):
    keys = []
    for item in text.split(","):
        item = item.strip()
        if not item:
            continue
        n, c = item.split(":", 1)
        keys.append((int(n), c))
    keys.sort()
    return keys


def new_machine(sim):
    try:
        return sim.Machine(0)
    except TypeError:
        return sim.Machine()


def out_bytes(m):
    for name in ("out", "tx", "output"):
        v = getattr(m, name, None)
        if v is not None:
            return v
    return b""


HEXPAD_KEYS = {"a": 0x04, "d": 0x06, "q": 0x12}   # meteor 1.1 の操作（HEXPAD の 4、6、A/D）


def feed(m, ch):
    """キーを 1 つ与える（1.1 の sample/tools は HEXPAD の press()。古い tools は受信の文字）"""
    if callable(getattr(m, "press", None)):
        m.press(HEXPAD_KEYS.get(ch.lower(), 0x05))
        return
    for name in ("feed", "send_key", "key"):
        f = getattr(m, name, None)
        if callable(f):
            f(ord(ch))
            return
    m.rx = ord(ch)


def rx_empty(m):
    if hasattr(m, "key_new"):
        return not m.key_new
    if hasattr(m, "rx_empty"):
        v = m.rx_empty
        return v() if callable(v) else bool(v)
    return getattr(m, "rx", None) is None


def run(sim, image, start, keys, quit_at):
    m = new_machine(sim)
    for a, b in image.items():
        m.mem[a] = b
    sentinel = getattr(sim, "SENTINEL", 0xFFF0)
    monitor = getattr(sim, "MONITOR", None)
    # モニタが jsr で呼んだのと同じ形にする（exit = rts は sentinel に戻る）
    ret = (sentinel - 1) & 0xFFFF
    s = getattr(m, "S", 0xFF)
    m.mem[0x100 + s] = ret >> 8
    s = (s - 1) & 0xFF
    m.mem[0x100 + s] = ret & 0xFF
    s = (s - 1) & 0xFF
    m.S = s
    m.PC = start
    pending = list(keys)
    quit_sent = False
    start_cyc = m.cyc
    while not getattr(m, "halted", False) and m.PC != sentinel and m.PC != monitor:
        m.step()
        if m.cyc - start_cyc > MAX_CYCLES:
            raise RuntimeError("終わりません（暴走？）")
        n = len(out_bytes(m))
        if pending and n >= pending[0][0] and rx_empty(m):
            feed(m, pending.pop(0)[1])
        elif not quit_sent and n >= quit_at and rx_empty(m):
            feed(m, "q")
            quit_sent = True
    return bytes(out_bytes(m)), m.cyc - start_cyc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tools", required=True)
    ap.add_argument("--ref", required=True)
    ap.add_argument("--keys", default=DEFAULT_KEYS, help="出力のバイト数:キー をカンマで並べる")
    ap.add_argument("--quit", type=int, default=DEFAULT_QUIT, help="この出力のバイト数で q を送る")
    ap.add_argument("dirs", nargs="+")
    a = ap.parse_args()
    sys.path.insert(0, os.path.abspath(a.tools))
    from asm65 import Asm
    import sim65c02 as sim

    keys = parse_keys(a.keys)
    ref = Asm().run(open(a.ref, encoding="utf-8").read())
    ref_img = dict(ref.out)
    rout, rcyc = run(sim, ref_img, ref.sym["start"], keys, a.quit)
    print(f"想定出力（{a.ref}）：出力 {len(rout)} バイト、{rcyc} サイクル")
    ok = True
    for d in a.dirs:
        base = None
        for f in os.listdir(d):
            if f.endswith(".bin"):
                base = os.path.join(d, f[:-4])
        if base is None:
            print(f"{d}: .bin がありません")
            ok = False
            continue
        sym = load_sym(base + ".sym")
        data = open(base + ".bin", "rb").read()
        if "start" not in sym:
            print(f"{d}: .sym に start がありません")
            ok = False
            continue
        org = sym["start"]
        img = {org + i: b for i, b in enumerate(data)}
        out, cyc = run(sim, img, org, keys, a.quit)
        same = out == rout
        ok = ok and same
        print(f"{d}: 出力 {len(out)} バイト、想定出力と{'一致' if same else '不一致'}；"
              f"{cyc} サイクル（想定出力 {rcyc}）、コード {len(data)} バイト（ファイル）")
        if not same:
            for i, (x, y) in enumerate(zip(out, rout)):
                if x != y:
                    print(f"  最初の違い：{i} バイト目 {out[max(0, i - 20):i + 20]!r} / 想定 {rout[max(0, i - 20):i + 20]!r}")
                    break
            else:
                print(f"  長さが違います：{len(out)} / {len(rout)}")
    print("結果：", "すべて一致" if ok else "不一致あり")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
