#!/usr/bin/env python3
"""Assemble both versions, run them on the emulator with the same key script, compare."""
import json, statistics, sys
from asm65 import Asm
from sim65c02 import Machine, MONITOR

KEYS = {5: 'a', 12: 'A', 30: 'd', 60: 'x', 90: 'a', 140: 'a', 200: 'd', 260: 'D', 261: 'a'}
QUIT_FRAME = 600

def build(name):
    a = Asm().run(open(name + ".s", encoding="utf-8").read())
    return a

def run(name):
    a = build(name); sym = a.sym
    m = Machine()
    for addr, b in a.out.items(): m.mem[addr] = b
    m.PC = sym["start"]
    L = {sym[k]: k for k in ("bench_loop", "bench_frame_begin", "bench_upd", "bench_draw", "bench_frame_end")}
    frame = 0; startup = None; t = {}; spans = {"input": [], "update": [], "draw": [], "frame": []}
    def busy(): return m.cyc - m.irq_cycles
    steps = 0
    while not m.halted and m.PC != MONITOR:
        if m.PC in L and not m.in_irq:
            k = L[m.PC]
            if k == "bench_loop" and startup is None: startup = busy()
            if k == "bench_frame_begin":
                frame += 1; t = {"b": busy()}
                if frame in KEYS: m.rx = ord(KEYS[frame])
                if frame == QUIT_FRAME: m.rx = ord('q')
            elif k == "bench_upd" and "b" in t: t["u"] = busy(); spans["input"].append(t["u"] - t["b"])
            elif k == "bench_draw" and "u" in t: t["d"] = busy(); spans["update"].append(t["d"] - t["u"])
            elif k == "bench_frame_end" and "d" in t:
                e = busy(); spans["draw"].append(e - t["d"]); spans["frame"].append(e - t["b"]); t = {}
        m.step(); steps += 1
        if steps > 20_000_000: raise RuntimeError("runaway")
    code = sym["code_end"] - 0x300; data = sym["data_end"] - sym["code_end"]
    ram = sym["vars_end"] - sym["data_end"]
    return dict(name=name, out=bytes(m.out), frames=frame, startup=startup, spans=spans,
                irq_count=m.irq_count, irq_avg=m.irq_cycles / max(1, m.irq_count), irq_max=m.irq_max,
                code=code, data=data, ram=ram, lst=a)

def stats(v): return (min(v), statistics.mean(v), max(v)) if v else (0, 0, 0)

def main():
    rs = [run(n) for n in ("meteor_O0", "meteor_O1", "meteor_O2", "meteor_Os", "meteor_hand")]
    same = all(r["out"] == rs[0]["out"] for r in rs)
    res = {"same_output": same, "output_tail": rs[0]["out"][-40:].decode(errors="replace"),
           "output_len": len(rs[0]["out"])}
    for r in rs:
        res[r["name"]] = {"frames": r["frames"], "startup_cycles": r["startup"],
            "code_bytes": r["code"], "const_bytes": r["data"], "ram_bytes": r["ram"],
            "irq_count": r["irq_count"], "irq_avg": round(r["irq_avg"], 1), "irq_max": r["irq_max"],
            **{k + "_min_avg_max": [round(x, 1) for x in stats(v)] for k, v in r["spans"].items()}}
    print(json.dumps(res, ensure_ascii=False, indent=1))
    json.dump(res, open("bench_result.json", "w"), ensure_ascii=False, indent=1)
    if not same:
        a, b = rs[0]["out"], rs[1]["out"]
        i = next(i for i in range(min(len(a), len(b))) if a[i] != b[i]) if any(x != y for x, y in zip(a, b)) else min(len(a), len(b))
        print("DIFF at", i, a[i-20:i+20], b[i-20:i+20], len(a), len(b))

if __name__ == "__main__": main()
