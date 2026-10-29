#!/usr/bin/env python3
"""ax65 でコンパイルした meteor を、sample/tools のエミュレータで動かし、
手書きの想定出力（asm/meteor_O0.s を tools/asm65.py で組み立てたもの）と端末への出力を比べる。

キー入力は run_bench.py と同じ（KEYS、QUIT_FRAME）。コンパイルした版には bench_ のラベルがないので、
フレームの区切りは、.sym にある TICKS（1/60 秒ごとに増える）が変わった時点とする。
1 フレームのサイクル数は、TICKS が変わってから次に wai を実行するまで（割り込みの分を除く）。

使い方：python3 meteor_compare.py --tools <tools> --ref <meteor_O0.s> <出力フォルダ>...
"""
import argparse, os, re, statistics, sys

KEYS = {5: 'a', 12: 'A', 30: 'd', 60: 'x', 90: 'a', 140: 'a', 200: 'd', 260: 'D', 261: 'a'}
QUIT_FRAME = 600
KEYS_HIT = {5: 'a', 12: 'A', 30: 'd', 31: 'd', 32: 'D', 60: 'x', 90: 'a'}


def load_sym(path):
    sym = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"^\s*([A-Za-z_][\w@~:.]*)\s*=\s*\$([0-9A-Fa-f]+)", line)
            if m:
                sym[m.group(1)] = int(m.group(2), 16)
    return sym


def run(machine_cls, monitor, image, start, ticks_addr, keys, quit_frame, max_steps=60_000_000):
    m = machine_cls(0)
    for a, b in image.items():
        m.mem[a] = b
    m.PC = start
    last = None
    busy0 = None
    frames = []
    steps = 0
    while not m.halted and m.PC != monitor:
        was_waiting = m.waiting
        m.step()
        steps += 1
        if steps > max_steps:
            raise RuntimeError("終わりません（暴走？）")
        t = m.mem[ticks_addr] | (m.mem[ticks_addr + 1] << 8)
        if t != last:
            last = t
            if t in keys:
                m.rx = ord(keys[t])
            if t == quit_frame:
                m.rx = ord('q')
            busy0 = m.cyc - m.irq_cycles
        if m.waiting and not was_waiting and not m.in_irq and busy0 is not None:
            frames.append(m.cyc - m.irq_cycles - busy0)
            busy0 = None
    return bytes(m.out), frames


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tools", required=True)
    ap.add_argument("--ref", required=True)
    ap.add_argument("dirs", nargs="+")
    a = ap.parse_args()
    sys.path.insert(0, os.path.abspath(a.tools))
    from asm65 import Asm
    from sim65c02 import Machine, MONITOR

    ref = Asm().run(open(a.ref, encoding="utf-8").read())
    ref_img = dict(ref.out)
    results = {}
    for name, keys, qf in (("ideal", KEYS, QUIT_FRAME), ("hit", KEYS_HIT, 3000)):
        out, fr = run(Machine, MONITOR, ref_img, ref.sym["start"], ref.sym["TICKS"], keys, qf)
        results[("ref", name)] = (out, fr)
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
        org = sym.get("start", 0x300)
        img = {org + i: b for i, b in enumerate(data)}
        if "TICKS" not in sym:
            print(f"{d}: .sym に TICKS がありません")
            ok = False
            continue
        for name, keys, qf in (("ideal", KEYS, QUIT_FRAME), ("hit", KEYS_HIT, 3000)):
            out, fr = run(Machine, MONITOR, img, sym["start"], sym["TICKS"], keys, qf)
            rout, rfr = results[("ref", name)]
            same = out == rout
            ok = ok and same
            avg = round(statistics.mean(fr), 1) if fr else 0
            ravg = round(statistics.mean(rfr), 1) if rfr else 0
            print(f"{d} [{name}]: 出力 {len(out)} バイト、想定出力と{'一致' if same else '不一致'}；"
                  f"1 フレーム平均 {avg} サイクル（想定出力 {ravg}）、コード {len(data)} バイト（ファイル）")
            if not same:
                for i, (x, y) in enumerate(zip(out, rout)):
                    if x != y:
                        print(f"  最初の違い：{i} バイト目 {out[max(0,i-20):i+20]!r} / 想定 {rout[max(0,i-20):i+20]!r}")
                        break
                else:
                    print(f"  長さが違います：{len(out)} / {len(rout)}")
    print("結果：", "すべて一致" if ok else "不一致あり")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
