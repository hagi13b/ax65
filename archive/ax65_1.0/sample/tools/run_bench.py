#!/usr/bin/env python3
"""meteor の効果測定（ax65 仕様書 1.0 / サンプルと効果測定 1.0）

使い方:  python3 -I tools/run_bench.py            （パッケージのルートで実行）
  ../asm/*.s を組み立て（.bin .lst .sym.json を asm/ に出力）、エミュレータで実行して
  results/bench_result.json に結果を書く。

計測の条件:
  ・5つの版に同じキー操作を同じフレームで与える（KEYS、QUIT_FRAME）
  ・ideal  : ACIA は常に送信可能（CPU の処理だけを比べる）
  ・serial : ACIA は 1 文字ごとに 1042 サイクル（1MHz で 9600bps）送信中になる
  ・計測用ラベル bench_frame_begin / bench_upd / bench_draw / bench_frame_end の間のサイクル数。
    途中で入った割り込みの分は差し引く
"""
import json, os, statistics, sys
HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
from asm65 import Asm
from sim65c02 import Machine, MONITOR

ROOT = os.path.dirname(HERE)
ASM = os.path.join(ROOT, "asm"); RES = os.path.join(ROOT, "results")
VERSIONS = ["meteor_O0", "meteor_O1", "meteor_O2", "meteor_Os", "meteor_hand"]
KEYS = {5: 'a', 12: 'A', 30: 'd', 60: 'x', 90: 'a', 140: 'a', 200: 'd', 260: 'D', 261: 'a'}
QUIT_FRAME = 600
KEYS_HIT = {5: 'a', 12: 'A', 30: 'd', 31: 'd', 32: 'D', 60: 'x', 90: 'a'}   # 隕石に当たって終わる入力
FRAME = 16666                                   # 1MHz で 1/60 秒

def build(name):
    a = Asm().run(open(os.path.join(ASM, name + ".s"), encoding="utf-8").read())
    hi = max(a.out); data = bytes(a.out.get(i, 0) for i in range(a.org, hi + 1))
    open(os.path.join(ASM, name + ".bin"), "wb").write(data)
    with open(os.path.join(ASM, name + ".lst"), "w", encoding="utf-8") as f:
        f.write("addr  bytes      cyc   source\n")
        for addr, b, cs, raw in a.lst:
            show = addr is not None and (b or cs or raw.split(';')[0].strip().endswith(':') or '.res' in raw)
            f.write(f"{(f'{addr:04X}' if show else '    ')}  {b.hex(' ').upper():<10} {cs:<5} {raw}\n")
    json.dump(a.sym, open(os.path.join(ASM, name + ".sym.json"), "w"), indent=0)
    return a

def run(a, keys, quit_frame, tx_cycles):
    sym = a.sym; m = Machine(tx_cycles)
    for addr, b in a.out.items(): m.mem[addr] = b
    m.PC = sym["start"]
    L = {sym[k]: k for k in ("bench_loop", "bench_frame_begin", "bench_upd", "bench_draw", "bench_frame_end")}
    frame = 0; startup = None; t = {}; sp = {"input": [], "update": [], "draw": [], "frame": []}
    busy = lambda: m.cyc - m.irq_cycles
    steps = 0
    while not m.halted and m.PC != MONITOR:
        if m.PC in L and not m.in_irq:
            k = L[m.PC]
            if k == "bench_loop" and startup is None: startup = busy()
            if k == "bench_frame_begin":
                frame += 1; t = {"b": busy()}
                if frame in keys: m.rx = ord(keys[frame])
                if frame == quit_frame: m.rx = ord('q')
            elif k == "bench_upd" and "b" in t: t["u"] = busy(); sp["input"].append(t["u"] - t["b"])
            elif k == "bench_draw" and "u" in t: t["d"] = busy(); sp["update"].append(t["d"] - t["u"])
            elif k == "bench_frame_end" and "d" in t:
                e = busy(); sp["draw"].append(e - t["d"]); sp["frame"].append(e - t["b"]); t = {}
        m.step(); steps += 1
        if steps > 50_000_000: raise RuntimeError("runaway")
    st = lambda v: [min(v), round(statistics.mean(v), 1), max(v)] if v else [0, 0, 0]
    return dict(out=bytes(m.out), frames=frame, startup=startup,
                irq_count=m.irq_count, irq_avg=round(m.irq_cycles / max(1, m.irq_count), 1),
                **{k: st(v) for k, v in sp.items()},
                overrun=sum(1 for v in sp["frame"] if v + 40 > FRAME))

def main():
    os.makedirs(RES, exist_ok=True)
    res = {"conditions": {"keys": KEYS, "quit_frame": QUIT_FRAME, "frame_cycles": FRAME,
                          "serial_tx_cycles": 1042}, "versions": {}}
    outs = {}
    for v in VERSIONS:
        a = build(v); s = a.sym
        r_ideal = run(a, KEYS, QUIT_FRAME, 0)
        r_ser = run(a, KEYS, QUIT_FRAME, 1042)
        r_hit = run(a, KEYS_HIT, 3000, 0)
        outs[v] = (r_ideal["out"], r_ser["out"], r_hit["out"])
        res["versions"][v] = {
            "code_bytes": s["code_end"] - 0x300, "const_bytes": s["data_end"] - s["code_end"],
            "file_bytes": s["data_end"] - 0x300,
            "ideal": {k: r_ideal[k] for k in r_ideal if k != "out"},
            "serial": {k: r_ser[k] for k in ("frames", "startup", "frame", "overrun")},
            "hit_run_frames": r_hit["frames"]}
    base = outs[VERSIONS[0]]
    res["same_output"] = {"ideal": all(o[0] == base[0] for o in outs.values()),
                          "serial": all(o[1] == base[1] for o in outs.values()) and base[0] == base[1],
                          "hit": all(o[2] == base[2] for o in outs.values())}
    res["output_bytes"] = len(base[0]); res["final_line"] = base[0].decode().strip().split("\r")[-1]
    json.dump(res, open(os.path.join(RES, "bench_result.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print(json.dumps(res["same_output"]), res["output_bytes"], res["final_line"])
    print(f'{"version":12}{"code":>6}{"file":>6}{"start":>7}{"irq":>6}{"input":>8}{"update":>8}{"draw":>7}{"frame":>8}{"max":>6}{"ser_avg":>9}{"ser_max":>8}')
    for v, d in res["versions"].items():
        i = d["ideal"]; s = d["serial"]
        print(f'{v:12}{d["code_bytes"]:>6}{d["file_bytes"]:>6}{i["startup"]:>7}{i["irq_avg"]:>6}{i["input"][1]:>8}{i["update"][1]:>8}{i["draw"][1]:>7}{i["frame"][1]:>8}{i["frame"][2]:>6}{s["frame"][1]:>9}{s["frame"][2]:>8}')

if __name__ == "__main__": main()
