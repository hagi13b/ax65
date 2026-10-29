#!/usr/bin/env python3
"""meteor の効果測定（AX65 仕様書 1.1 / サンプルと効果測定 1.1）

使い方:  python3 -I tools/run_bench.py            （パッケージのルートで実行）
  ../asm/*.s を組み立て（.bin .lst .sym.json .woz を asm/ に出力）、エミュレータで実行して
  results/bench_result.json に結果を書く。

計測の条件:
  ・ターゲットは v2（実機：W65C02S 約 1.8425MHz、UART 19200bps、HEXPAD、割り込みなし）
  ・5つの版に同じキー操作（HEXPAD のキーコード）を同じフレームで与える（KEYS、QUIT_FRAME）
  ・ideal  : UART は常に送信可能（CPU の処理だけを比べる）
  ・serial : UART は 1 文字ごとに 960 サイクル（約 1.8425MHz で 19200bps）送信中になる
  ・計測用ラベル bench_frame_begin / bench_upd / bench_draw / bench_frame_end の間のサイクル数
"""
import json, os, statistics, sys
HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
from asm65 import Asm
from sim65c02 import Machine

ROOT = os.path.dirname(HERE)
ASM = os.path.join(ROOT, "asm"); RES = os.path.join(ROOT, "results")
VERSIONS = ["meteor_O0", "meteor_O1", "meteor_O2", "meteor_Os", "meteor_hand"]
LEFT, RIGHT, OTHER, QUIT = 0x04, 0x06, 0x05, 0x12   # 4 のキー、6 のキー、使わないキー（5）、A/D のキー
KEYS = {5: LEFT, 12: LEFT, 30: RIGHT, 60: OTHER, 90: LEFT, 140: LEFT, 200: RIGHT, 260: RIGHT, 261: LEFT}
QUIT_FRAME = 600
KEYS_HIT = {5: LEFT, 12: LEFT, 30: RIGHT, 31: RIGHT, 32: RIGHT, 60: OTHER, 90: LEFT}   # 隕石に当たって終わる入力
FRAME = 30708                                   # 約 1.8425MHz で 1/60 秒
TX = 960                                        # 19200bps の 1 文字（10 ビット）のサイクル数
LOAD = 0x0200

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
    with open(os.path.join(ASM, name + ".woz"), "w") as f:      # HEXMON に打ち込むダンプ（8 バイトずつ）
        for i in range(0, len(data), 8):
            f.write(f"{a.org + i:04X}: " + " ".join(f"{b:02X}" for b in data[i:i + 8]) + "\n")
    return a

def run(a, keys, quit_frame, tx_cycles):
    sym = a.sym; m = Machine(tx_cycles)
    for addr, b in a.out.items(): m.mem[addr] = b
    m.start(sym["start"])
    L = {sym[k]: k for k in ("bench_loop", "bench_frame_begin", "bench_upd", "bench_draw", "bench_frame_end")}
    frame = 0; startup = None; t = {}; sp = {"input": [], "update": [], "draw": [], "frame": []}
    busy = lambda: m.cyc
    steps = 0
    while not m.halted:
        if m.PC in L:
            k = L[m.PC]
            if k == "bench_loop" and startup is None: startup = busy()
            if k == "bench_frame_begin":
                frame += 1; t = {"b": busy()}
                if frame in keys: m.press(keys[frame])
                if frame == quit_frame: m.press(QUIT)
            elif k == "bench_upd" and "b" in t: t["u"] = busy(); sp["input"].append(t["u"] - t["b"])
            elif k == "bench_draw" and "u" in t: t["d"] = busy(); sp["update"].append(t["d"] - t["u"])
            elif k == "bench_frame_end" and "d" in t:
                e = busy(); sp["draw"].append(e - t["d"]); sp["frame"].append(e - t["b"]); t = {}
        m.step(); steps += 1
        if steps > 50_000_000: raise RuntimeError("runaway")
    st = lambda v: [min(v), round(statistics.mean(v), 1), max(v)] if v else [0, 0, 0]
    return dict(out=bytes(m.out), frames=frame, startup=startup, stack_bytes=m.s_start - m.s_min + 2,
                **{k: st(v) for k, v in sp.items()},
                overrun=sum(1 for v in sp["frame"] if v + 40 > FRAME))

def main():
    os.makedirs(RES, exist_ok=True)
    res = {"conditions": {"keys": KEYS, "quit_frame": QUIT_FRAME, "frame_cycles": FRAME,
                          "serial_tx_cycles": TX}, "versions": {}}
    outs = {}
    for v in VERSIONS:
        a = build(v); s = a.sym
        r_ideal = run(a, KEYS, QUIT_FRAME, 0)
        r_ser = run(a, KEYS, QUIT_FRAME, TX)
        r_hit = run(a, KEYS_HIT, 3000, 0)
        outs[v] = (r_ideal["out"], r_ser["out"], r_hit["out"])
        res["versions"][v] = {
            "code_bytes": s["code_end"] - LOAD, "const_bytes": s["data_end"] - s["code_end"],
            "file_bytes": s["data_end"] - LOAD, "keys_to_type": 3 * (s["data_end"] - LOAD),
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
    print(f'{"version":12}{"code":>6}{"file":>6}{"start":>7}{"stack":>6}{"input":>8}{"update":>8}{"draw":>7}{"frame":>8}{"max":>6}{"ser_avg":>9}{"ser_max":>8}')
    for v, d in res["versions"].items():
        i = d["ideal"]; s = d["serial"]
        print(f'{v:12}{d["code_bytes"]:>6}{d["file_bytes"]:>6}{i["startup"]:>7}{i["stack_bytes"]:>6}{i["input"][1]:>8}{i["update"][1]:>8}{i["draw"][1]:>7}{i["frame"][1]:>8}{i["frame"][2]:>6}{s["frame"][1]:>9}{s["frame"][2]:>8}')

if __name__ == "__main__": main()
