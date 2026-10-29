#!/usr/bin/env python3
"""cc65 版 meteor の測定（ax65 版と同じ条件）

使い方:  python3 -I tools/run_cc65.py cc65/out        （パッケージのルートで実行）
  <フォルダ>/meteor_<設定>.bin（と、あれば .map）を読み、ax65 版と同じキー操作・同じ条件で実行する。
  計測の目印は $7F00+n への書き込み（n: 0=frame_begin, 1=upd, 2=draw, 3=frame_end, 4=loop）。
  目印の命令（stz abs、4 サイクル）は、各区間から差し引く。
  結果は results/cc65_result.json に書く。ax65 の -O2 版と端末への出力を照合する。
"""
import glob, json, os, re, statistics, sys
HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
from sim65c02 import Machine
from run_bench import KEYS, QUIT_FRAME, KEYS_HIT, QUIT, TX, LOAD, ROOT, RES, build, run as run_ax65

MARK_CYC = 4

def seg_sizes(mapfile):
    sizes = {}
    if not os.path.exists(mapfile): return sizes
    txt = open(mapfile, encoding="latin-1").read()
    m = re.search(r"Segment list:\s*\n-+\n.*?\n-+\n(.*?)\n\n", txt, re.S)
    if m:
        for line in m.group(1).splitlines():
            p = line.split()
            if len(p) >= 4: sizes[p[0]] = int(p[3], 16)
    return sizes

def run_bin(path, keys, quit_frame, tx):
    m = Machine(tx)
    data = open(path, "rb").read()
    m.mem[LOAD:LOAD + len(data)] = data
    m.start(LOAD)
    st = {"frame": 0, "startup": None, "t": {}, "sp": {"input": [], "update": [], "draw": [], "frame": []}}
    busy = lambda: m.cyc
    def mark(n):
        t = st["t"]; c = busy()
        if n == 4 and st["startup"] is None: st["startup"] = c
        if n == 0:
            st["frame"] += 1; st["t"] = {"b": c}
            if st["frame"] in keys: m.press(keys[st["frame"]])
            if st["frame"] == quit_frame: m.press(QUIT)
        elif n == 1 and "b" in t: t["u"] = c; st["sp"]["input"].append(c - t["b"] - MARK_CYC)
        elif n == 2 and "u" in t: t["d"] = c; st["sp"]["update"].append(c - t["u"] - MARK_CYC)
        elif n == 3 and "d" in t:
            st["sp"]["draw"].append(c - t["d"] - MARK_CYC)
            st["sp"]["frame"].append(c - t["b"] - 3 * MARK_CYC); st["t"] = {}
    m.on_mark = mark
    steps = 0
    while not m.halted:
        m.step(); steps += 1
        if steps > 80_000_000: raise RuntimeError("runaway")
    f = lambda v: [min(v), round(statistics.mean(v), 1), max(v)] if v else [0, 0, 0]
    return dict(out=bytes(m.out), frames=st["frame"], startup=st["startup"],
                stack_bytes=m.s_start - m.s_min + 2, **{k: f(v) for k, v in st["sp"].items()})

def main():
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "cc65", "out")
    ref = build("meteor_O2")
    ref_out = (run_ax65(ref, KEYS, QUIT_FRAME, 0)["out"], run_ax65(ref, KEYS, QUIT_FRAME, TX)["out"],
               run_ax65(ref, KEYS_HIT, 3000, 0)["out"])
    res = {}
    for b in sorted(glob.glob(os.path.join(folder, "meteor_*.bin"))):
        name = os.path.basename(b)[:-4]
        i = run_bin(b, KEYS, QUIT_FRAME, 0); s = run_bin(b, KEYS, QUIT_FRAME, TX); h = run_bin(b, KEYS_HIT, 3000, 0)
        seg = seg_sizes(b[:-4] + ".map")
        res[name] = {"file_bytes": os.path.getsize(b), "segments": seg,
                     "same_as_ax65": [i["out"] == ref_out[0], s["out"] == ref_out[1], h["out"] == ref_out[2]],
                     "ideal": {k: v for k, v in i.items() if k != "out"},
                     "serial": {k: s[k] for k in ("frames", "startup", "frame")}, "hit_frames": h["frames"]}
        print(f'{name:16} file {res[name]["file_bytes"]:5}  same {res[name]["same_as_ax65"]}  '
              f'frame {i["frame"][1]:8}  input {i["input"][1]:6} update {i["update"][1]:7} draw {i["draw"][1]:7} '
              f'stack {i["stack_bytes"]:4} start {i["startup"]}  serial {s["frame"][1]}')
    os.makedirs(RES, exist_ok=True)
    json.dump(res, open(os.path.join(RES, "cc65_result.json"), "w"), indent=1)

if __name__ == "__main__": main()
