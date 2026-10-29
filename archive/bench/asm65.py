#!/usr/bin/env python3
"""Minimal two-pass 65C02 assembler for the ax65 benchmark.
Syntax: labels `name:`, local labels `@name` (scoped to previous global label),
`name = expr`, `.org`, `.byte`, `.word`, `.res`. Comments start with `;`.
Outputs: binary (from first .org), listing with cycle counts, JSON symbols."""
import re, sys, json

# opcode table: (mnemonic, mode) -> (opcode, cycles, page_penalty)
OPS = {}
def op(m, mode, code, cyc, pen=False): OPS[(m, mode)] = (code, cyc, pen)
T = """
brk imp 00 7|ora izx 01 6|tsb zp 04 5|ora zp 05 3|asl zp 06 5|php imp 08 3|ora imm 09 2|asl acc 0A 2|tsb abs 0C 6|ora abs 0D 4|asl abs 0E 6
bpl rel 10 2|ora izy 11 5*|ora izp 12 5|trb zp 14 5|ora zpx 15 4|asl zpx 16 6|clc imp 18 2|ora aby 19 4*|inc acc 1A 2|trb abs 1C 6|ora abx 1D 4*|asl abx 1E 6*
jsr abs 20 6|and izx 21 6|bit zp 24 3|and zp 25 3|rol zp 26 5|plp imp 28 4|and imm 29 2|rol acc 2A 2|bit abs 2C 4|and abs 2D 4|rol abs 2E 6
bmi rel 30 2|and izy 31 5*|and izp 32 5|bit zpx 34 4|and zpx 35 4|rol zpx 36 6|sec imp 38 2|and aby 39 4*|dec acc 3A 2|bit abx 3C 4*|and abx 3D 4*|rol abx 3E 6*
rti imp 40 6|eor izx 41 6|eor zp 45 3|lsr zp 46 5|pha imp 48 3|eor imm 49 2|lsr acc 4A 2|jmp abs 4C 3|eor abs 4D 4|lsr abs 4E 6
bvc rel 50 2|eor izy 51 5*|eor izp 52 5|eor zpx 55 4|lsr zpx 56 6|cli imp 58 2|eor aby 59 4*|phy imp 5A 3|eor abx 5D 4*|lsr abx 5E 6*
rts imp 60 6|adc izx 61 6|stz zp 64 3|adc zp 65 3|ror zp 66 5|pla imp 68 4|adc imm 69 2|ror acc 6A 2|jmp ind 6C 6|adc abs 6D 4|ror abs 6E 6
bvs rel 70 2|adc izy 71 5*|adc izp 72 5|stz zpx 74 4|adc zpx 75 4|ror zpx 76 6|sei imp 78 2|adc aby 79 4*|ply imp 7A 4|jmp iax 7C 6|adc abx 7D 4*|ror abx 7E 6*
bra rel 80 3|sta izx 81 6|sty zp 84 3|sta zp 85 3|stx zp 86 3|dey imp 88 2|bit imm 89 2|txa imp 8A 2|sty abs 8C 4|sta abs 8D 4|stx abs 8E 4
bcc rel 90 2|sta izy 91 6|sta izp 92 5|sty zpx 94 4|sta zpx 95 4|stx zpy 96 4|tya imp 98 2|sta aby 99 5|txs imp 9A 2|stz abs 9C 4|sta abx 9D 5|stz abx 9E 5
ldy imm A0 2|lda izx A1 6|ldx imm A2 2|ldy zp A4 3|lda zp A5 3|ldx zp A6 3|tay imp A8 2|lda imm A9 2|tax imp AA 2|ldy abs AC 4|lda abs AD 4|ldx abs AE 4
bcs rel B0 2|lda izy B1 5*|lda izp B2 5|ldy zpx B4 4|lda zpx B5 4|ldx zpy B6 4|clv imp B8 2|lda aby B9 4*|tsx imp BA 2|ldy abx BC 4*|lda abx BD 4*|ldx aby BE 4*
cpy imm C0 2|cmp izx C1 6|cpy zp C4 3|cmp zp C5 3|dec zp C6 5|iny imp C8 2|cmp imm C9 2|dex imp CA 2|wai imp CB 3|cpy abs CC 4|cmp abs CD 4|dec abs CE 6
bne rel D0 2|cmp izy D1 5*|cmp izp D2 5|cmp zpx D5 4|dec zpx D6 6|cld imp D8 2|cmp aby D9 4*|phx imp DA 3|stp imp DB 3|cmp abx DD 4*|dec abx DE 7
cpx imm E0 2|sbc izx E1 6|cpx zp E4 3|sbc zp E5 3|inc zp E6 5|inx imp E8 2|sbc imm E9 2|nop imp EA 2|cpx abs EC 4|sbc abs ED 4|inc abs EE 6
beq rel F0 2|sbc izy F1 5*|sbc izp F2 5|sbc zpx F5 4|inc zpx F6 6|sed imp F8 2|sbc aby F9 4*|plx imp FA 4|sbc abx FD 4*|inc abx FE 7
"""
for item in T.replace("\n", "|").split("|"):
    item = item.strip()
    if not item: continue
    m, mode, code, cyc = item.split()
    op(m, mode, int(code, 16), int(cyc.rstrip("*")), cyc.endswith("*"))
MNEMS = {m for (m, _) in OPS}
SIZE = {"imp":1,"acc":1,"imm":2,"zp":2,"zpx":2,"zpy":2,"izx":2,"izy":2,"izp":2,"rel":2,"abs":3,"abx":3,"aby":3,"ind":3,"iax":3}
ZPOF = {"abs":"zp","abx":"zpx","aby":"zpy"}

class Asm:
    def __init__(s): s.sym = {}; s.glob = ""
    def name(s, n):
        return s.glob + n if n.startswith("@") else n
    def ev(s, e, strict):
        e = e.strip()
        def num(m):
            t = m.group(0)
            if t.startswith("$"): return str(int(t[1:], 16))
            if t.startswith("0b"): return str(int(t[2:], 2))
            return t
        def ch(m): return str(ord(m.group(1)))
        x = re.sub(r"'(.)'", ch, e)
        x = re.sub(r"\$[0-9A-Fa-f]+|0b[01]+", num, x)
        def nm(m):
            n = s.name(m.group(0))
            if n in s.sym: return str(s.sym[n])
            if strict: raise ValueError("undefined symbol " + n)
            return "0"
        x = re.sub(r"@?[A-Za-z_][A-Za-z0-9_]*", nm, x)
        known = all(s.name(n) in s.sym for n in re.findall(r"@?[A-Za-z_][A-Za-z0-9_]*", re.sub(r"'(.)'", "", re.sub(r"\$[0-9A-Fa-f]+|0b[01]+", "", e))))
        return int(eval(x, {}, {})), known
    def operand(s, m, t, strict):
        t = t.strip()
        if m in ("bpl","bmi","bvc","bvs","bcc","bcs","bne","beq","bra"):
            return "rel", t
        if t == "" : return "imp", None
        if t.lower() == "a": return "acc", None
        if t.startswith("#"):
            x = t[1:].strip()
            return "imm", x
        mm = re.match(r"^\((.*),\s*[xX]\)$", t)
        if mm: return ("iax" if m == "jmp" else "izx"), mm.group(1)
        mm = re.match(r"^\((.*)\)\s*,\s*[yY]$", t)
        if mm: return "izy", mm.group(1)
        mm = re.match(r"^\((.*)\)$", t)
        if mm: return ("ind" if m == "jmp" else "izp"), mm.group(1)
        mm = re.match(r"^(.*),\s*([xXyY])$", t)
        if mm: return ("abx" if mm.group(2).lower()=="x" else "aby"), mm.group(1)
        return "abs", t
    def immval(s, x, strict):
        if x.startswith("<"): v,k = s.ev(x[1:], strict); return v & 0xFF
        if x.startswith(">"): v,k = s.ev(x[1:], strict); return (v >> 8) & 0xFF
        v,k = s.ev(x, strict); return v & 0xFF
    def run(s, src):
        lines = src.split("\n")
        for p in (1, 2):
            s.glob = ""; pc = 0; out = {}; lst = []; s.modes = getattr(s, "modes", {})
            org0 = None
            for ln, raw in enumerate(lines):
                code = raw.split(";", 1)[0]
                code = code.rstrip()
                body = code.strip()
                labaddr = None
                mm = re.match(r"^(@?[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$", body)
                if mm:
                    v,k = s.ev(mm.group(2), p == 2)
                    if p == 2 or k: s.sym[mm.group(1)] = v
                    else: s.sym.pop(mm.group(1), None)
                    lst.append((None, b"", "", raw)); continue
                mm = re.match(r"^(@?[A-Za-z_][A-Za-z0-9_]*):\s*(.*)$", body)
                if mm:
                    n = mm.group(1)
                    if not n.startswith("@"): s.glob = n
                    s.sym[s.name(n)] = pc; body = mm.group(2).strip()
                if not body:
                    lst.append((pc if mm else None, b"", "", raw)); continue
                parts = body.split(None, 1); m = parts[0].lower(); arg = parts[1] if len(parts) > 1 else ""
                if m == ".org":
                    pc,_ = s.ev(arg, True); org0 = pc if org0 is None else org0
                    lst.append((None, b"", "", raw)); continue
                if m in (".byte", ".word"):
                    data = bytearray()
                    for it in re.findall(r'"[^"]*"|[^,]+', arg):
                        it = it.strip()
                        if it.startswith('"'): data += it[1:-1].encode()
                        else:
                            if m == ".byte": data.append(s.immval(it, p == 2))
                            else:
                                v,_ = s.ev(it, p == 2); data += bytes([v & 0xFF, (v >> 8) & 0xFF])
                    for i, b in enumerate(data): out[pc + i] = b
                    lst.append((pc, bytes(data), "", raw)); pc += len(data); continue
                if m == ".res":
                    v,_ = s.ev(arg, True); lst.append((pc, b"", "", raw)); pc += v; continue
                if m not in MNEMS: raise SyntaxError(f"line {ln+1}: unknown {m}")
                mode, x = s.operand(m, arg, p == 2)
                key = (ln,)
                if p == 1:
                    if mode in ZPOF and (m, ZPOF[mode]) in OPS:
                        v, known = s.ev(x, False)
                        if known and v < 256: mode = ZPOF[mode]
                    s.modes[key] = mode
                else:
                    mode = s.modes[key]
                if (m, mode) not in OPS: raise SyntaxError(f"line {ln+1}: bad mode {m} {mode}")
                opc, cyc, pen = OPS[(m, mode)]
                b = bytearray([opc])
                if mode == "rel":
                    v,_ = s.ev(x, p == 2); d = v - (pc + 2)
                    if p == 2 and not -128 <= d <= 127: raise ValueError(f"line {ln+1}: branch out of range ({d}) {raw.strip()}")
                    b.append(d & 0xFF)
                    cs = f"{cyc}/{cyc+1}" if m != "bra" else "3"
                elif mode == "imm": b.append(s.immval(x, p == 2)); cs = str(cyc)
                elif SIZE[mode] == 2:
                    v,_ = s.ev(x, p == 2); b.append(v & 0xFF); cs = str(cyc)
                elif SIZE[mode] == 3:
                    v,_ = s.ev(x, p == 2); b += bytes([v & 0xFF, (v >> 8) & 0xFF]); cs = str(cyc) + ("*" if pen else "")
                else: cs = str(cyc)
                if pen and SIZE[mode] == 2: cs += "*"
                for i, bb in enumerate(b): out[pc + i] = bb
                lst.append((pc, bytes(b), cs, raw)); pc += len(b)
        s.out = out; s.lst = lst; s.org = org0
        return s

def main():
    src, base = sys.argv[1], sys.argv[2]
    a = Asm().run(open(src, encoding="utf-8").read())
    hi = max(a.out); data = bytes(a.out.get(i, 0) for i in range(a.org, hi + 1))
    open(base + ".bin", "wb").write(data)
    with open(base + ".lst", "w", encoding="utf-8") as f:
        f.write("addr  bytes      cyc   source\n")
        for addr, b, cs, raw in a.lst:
            ad = f"{addr:04X}" if addr is not None and (b or cs or raw.strip().split(';')[0].strip().endswith(':') or '.res' in raw) else "    "
            f.write(f"{ad}  {b.hex(' ').upper():<10} {cs:<5} {raw}\n")
    json.dump({k: v for k, v in a.sym.items()}, open(base + ".sym.json", "w"), indent=0)
    print(f"{src}: {len(data)} bytes ${a.org:04X}-${hi:04X}")

if __name__ == "__main__": main()
