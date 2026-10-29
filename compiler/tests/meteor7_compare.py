#!/usr/bin/env python3
"""ax65 でコンパイルした meteor7（7セグメント版）と、想定出力 asm/meteor7_O2.s を、同じ自動の遊び手で動かして、表示の変化と点数を比べる。

使い方：python3 -I tests/meteor7_compare.py <sample/tools> <sample/asm/meteor7_O2.s> <build/meteor7_O2/meteor7>   （拡張子なし。.bin と .sym を読む）
"""
import sys, os, re
TOOLS=sys.argv[1]; sys.path.insert(0, TOOLS)
from sim65c02 import Machine
from asm65 import Asm
REACT=25
def run(img, sym):
    m=Machine(0)
    for a,b in img.items(): m.mem[a]=b
    m.start(sym["start"]); MET,PL,SC=sym["METEOR"],sym["PLAYER"],sym["SCORE"]
    seen=0; finished=False
    while not m.halted:
        if m.PC==sym["poll_key"]:
            p=m.mem[PL]
            if m.mem[MET+p] and not m.key_new:
                seen+=1
                if seen>=REACT:
                    left=next((d for d in range(1,6) if p+d<=5 and not m.mem[MET+p+d]),None)
                    right=next((d for d in range(1,6) if p-d>=0 and not m.mem[MET+p-d]),None)
                    if right is not None and (left is None or right<=left): m.press(0x06)
                    elif left is not None: m.press(0x04)
                    seen=0
            else: seen=0
            if m.display().startswith("--") and not m.key_new: finished=True; m.press(0x00)
        m.step()
        if m.cyc>600*1_842_500: raise RuntimeError("runaway")
    log=[]
    for c,d in m.digit_log:
        if not log or log[-1]!=d: log.append(d)
    return log, f'{m.mem[SC+1]:02X}{m.mem[SC]:02X}', m.cyc, finished
ref=Asm().run(open(sys.argv[2],encoding='utf-8').read())
r=run(dict(ref.out), ref.sym)
sym={}
for line in open(sys.argv[3]+'.sym',encoding='utf-8'):
    mm=re.match(r"^\s*([A-Za-z_][\w@~:.]*)\s*=\s*\$([0-9A-Fa-f]+)",line)
    if mm: sym[mm.group(1)]=int(mm.group(2),16)
data=open(sys.argv[3]+'.bin','rb').read()
img={sym['start']+i:b for i,b in enumerate(data)}
c=run(img,sym)
print('ref: displays',len(r[0]),'score',r[1],'cyc',r[2],'fin',r[3])
print('cmp: displays',len(c[0]),'score',c[1],'cyc',c[2],'fin',c[3], 'size',len(data))
print('SAME' if r[0]==c[0] and r[1]==c[1] else 'DIFF')
