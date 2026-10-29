#!/usr/bin/env python3
"""meteor7（7セグメント版）の動作確認（AX65 仕様書 1.1 / サンプルと効果測定 1.1）

使い方:  python3 -I tools/run_meteor7.py            （sample のフォルダで実行）
  asm/meteor7_O2.s を組み立て（.bin .lst .sym.json .woz を asm/ に出力）、エミュレータで遊ばせる。
  遊ぶのは簡単なプログラム：自分の列に隕石が来たら、REACT 回（10ms ごと）たってから空いている列へ動く。
  隕石が速くなると間に合わなくなって終わる。終わったら表示を確かめ、キーを 1 つ押してモニタへ戻す。
  表示の変化は results/meteor7_display.txt に書く。
"""
import json, os, sys
HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
from sim65c02 import Machine
from run_bench import build, ASM, RES, LOAD

REACT = 25                                      # 反応までの回数（25 × 10ms = 0.25 秒）

def main():
    a = build("meteor7_O2"); sym = a.sym
    size = len(open(os.path.join(ASM, "meteor7_O2.bin"), "rb").read())   # .woz は build が書く
    m = Machine(0)
    for addr, b in a.out.items(): m.mem[addr] = b
    m.start(sym["start"])
    MET, PL, SC = sym["METEOR"], sym["PLAYER"], sym["SCORE"]
    seen = 0; polls = 0; moves = 0; steps = 0; finished = False
    while not m.halted:
        if m.PC == sym["poll_key"]:
            polls += 1
            p = m.mem[PL]
            if m.mem[MET + p] and not m.key_new:
                seen += 1
                if seen >= REACT:               # 反応して、空いている近い列へ動く
                    left = next((d for d in range(1, 6) if p + d <= 5 and not m.mem[MET + p + d]), None)
                    right = next((d for d in range(1, 6) if p - d >= 0 and not m.mem[MET + p - d]), None)
                    if right is not None and (left is None or right <= left): m.press(0x06)
                    elif left is not None: m.press(0x04)
                    moves += 1; seen = 0
            else:
                seen = 0
            if m.display().startswith("--") and not m.key_new:
                finished = True; m.press(0x00)  # 点数が出たら、何かキーを押す
        if m.PC == sym["do_step"]: steps += 1
        m.step()
        if m.cyc > 600 * 1_842_500: raise RuntimeError("did not finish in 600 s")
    score = f'{m.mem[SC + 1]:02X}{m.mem[SC]:02X}'
    log = []
    for c, d in m.digit_log:
        if not log or log[-1][1] != d: log.append((c, d))
    os.makedirs(RES, exist_ok=True)
    with open(os.path.join(RES, "meteor7_display.txt"), "w") as f:
        for c, d in log: f.write(f"{c / 1_842_500:9.3f}s  [{d}]\n")
    res = {"bytes": size, "keys_to_type": 3 * size, "score": score, "final_display": m.display(),
           "steps": steps, "polls": polls, "moves": moves, "seconds": round(m.cyc / 1_842_500, 1),
           "stack_bytes": m.s_start - m.s_min + 2, "returned_to_monitor": m.halted and finished}
    json.dump(res, open(os.path.join(RES, "meteor7_result.json"), "w"), indent=1)
    print(json.dumps(res, ensure_ascii=False))

if __name__ == "__main__": main()
