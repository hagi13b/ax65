#!/usr/bin/env python3
"""disasm（逆アセンブラ。sample/disasm）の動作確認（AX65 仕様書 1.1）

使い方:  python3 -I tools/run_disasm.py <disasm.bin> [端末の出力を書くファイル]     （sample のフォルダで実行）
  コンパイルした disasm を $0200 に読み込み、エミュレータの UART からコマンドを打ち込んで、
  端末への出力を、ここで別に書いた参照用の逆アセンブラ（Apple II の L コマンドの形）の出力と 1 文字ずつ比べる。
  試すこと：
    - $3000 からの、全 256 個の命令コードを並べたデータ（3000L と、続きの L）
    - 前向き・後ろ向きの相対分岐、BBR・BBS、番地の 5 桁以上の入力、小文字、バックスペース、まちがったコマンド
    - $FFF8 から $FFFF を越えて $0000 へ一周する場合
    - Q で HEXMON へ戻ること
"""
import os, sys
HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
from sim65c02 import Machine
from asm65 import OPS

# ---- 参照用の逆アセンブラ（asm65 の命令表 ＋ W65C02S の RMB・SMB・BBR・BBS） ----
REF = {code: (m.upper(), mode) for (m, mode), (code, _, _) in OPS.items()}
for b in range(8):
    REF[0x07 + b * 16] = (f"RMB{b}", "zp")
    REF[0x87 + b * 16] = (f"SMB{b}", "zp")
    REF[0x0F + b * 16] = (f"BBR{b}", "zpr")
    REF[0x8F + b * 16] = (f"BBS{b}", "zpr")
LEN = {"imp": 1, "acc": 1, "imm": 2, "zp": 2, "zpx": 2, "zpy": 2, "izx": 2, "izy": 2, "izp": 2, "rel": 2,
       "abs": 3, "abx": 3, "aby": 3, "ind": 3, "iax": 3, "zpr": 3, "bad": 1}

def ref_line(mem, pc):
    """1 命令を Apple II の L コマンドの形にする。(文字列, 次の番地)"""
    op = mem[pc]; name, mode = REF.get(op, ("???", "bad")); n = LEN[mode]
    b = [mem[(pc + i) & 0xFFFF] for i in range(3)]
    field = "".join(f"{b[i]:02X} " if i < n else "   " for i in range(3)) + "   "
    def tgt(nxt, off): return f"${(nxt + (off - 256 if off > 127 else off)) & 0xFFFF:04X}"
    w = f"${b[2]:02X}{b[1]:02X}"; z = f"${b[1]:02X}"
    opr = {"imp": "", "acc": "", "bad": "", "imm": "#" + z, "zp": z, "zpx": z + ",X", "zpy": z + ",Y",
           "abs": w, "abx": w + ",X", "aby": w + ",Y", "ind": f"({w})", "izx": f"({z},X)", "izy": f"({z}),Y",
           "izp": f"({z})", "iax": f"({w},X)", "rel": tgt(pc + 2, b[1]), "zpr": z + "," + tgt(pc + 3, b[2])}[mode]
    text = f"\r\n{pc:04X}-   {field}{name}" + (" " * (6 - len(name)) + opr if opr else "")
    return text, (pc + n) & 0xFFFF

def ref_list(mem, pc, count=20):
    out = ""
    for _ in range(count):
        t, pc = ref_line(mem, pc); out += t
    return out, pc

# ---- テスト ----
def main():
    m = Machine(0)
    code = open(sys.argv[1], "rb").read()
    m.mem[0x0200:0x0200 + len(code)] = code

    data = bytearray()                              # $3000：命令コード 0〜255 を順に並べ、オペランドは決まった計算で作る
    for op in range(256):
        n = LEN[REF.get(op, ("", "bad"))[1]]
        data.append(op)
        for i in range(1, n): data.append((op * 7 + i * 0x35) & 0xFF)
    m.mem[0x3000:0x3000 + len(data)] = data
    m.mem[0x4000:0x4006] = bytes([0xD0, 0xFE, 0x80, 0x80, 0x0F, 0x12])   # bne * / bra 後ろへ / BBR0（次の行と合わせて）
    m.mem[0x4006:0x4008] = bytes([0x7F, 0x34])
    for a in range(0xFFF8, 0x10000): m.mem[a] = 0x4C if a & 1 == 0 else 0x12
    m.mem[0x0000:0x0010] = bytes(range(0xA9, 0xB9))

    expect = "\r\nAX65 DISASSEMBLER  (ADDR L / L / Q)"
    keys = ""
    def cmd(typed, echo, body):
        nonlocal keys, expect
        keys += typed + "\r"; expect += "\r\n*" + echo + body

    # 1. 全命令コード：3000L と続きの L（20 × 14 = 280 命令で、256 個の命令コードをすべて通る）
    pc = 0x3000; body, pc = ref_list(m.mem, pc); cmd("3000L", "3000L", body)
    for _ in range(13):
        body, pc = ref_list(m.mem, pc); cmd("L", "L", body)
    # 2. 分岐・BBR・BBS。5 桁の番地（最後の 4 桁が使われる）、小文字、L を 2 つ
    body, pc = ref_list(m.mem, 0x4000, 40); cmd("14000ll", "14000LL", body)
    # 3. バックスペース：「5」を打ってから消して 4000L
    body, _ = ref_list(m.mem, 0x4000); cmd("5\x084000L", "5\x08 \x084000L", body)
    # 4. $FFFF を越えて $0000 へ一周する
    body, _ = ref_list(m.mem, 0xFFF8); cmd("FFF8L", "FFF8L", body)
    # 5. まちがったコマンドと、番地だけの行（ベルと ?）
    cmd("X", "X", "\x07\r\n?")
    cmd("1234", "1234", "\x07\r\n?")
    # 6. 空の行は何もしない。Q で HEXMON へ戻る
    cmd("", "", "")
    keys += "q\r"; expect += "\r\n*Q\r\n"

    m.start(0x0200)                                 # HEXMON で 0200 → JMP と同じ
    q = list(keys)
    while not m.halted:
        if m.rx is None and q: m.rx = ord(q.pop(0))
        m.step()
        if m.cyc > 200_000_000: raise RuntimeError("did not finish")
    got = m.out.decode("latin-1")
    if len(sys.argv) > 2:
        open(sys.argv[2], "w", encoding="latin-1", newline="").write(got)
    if got == expect:
        print(f"OK: {len(got)} 文字が一致（{got.count(chr(10))} 行）、{m.cyc} サイクル、"
              f"スタック {m.s_start - m.s_min + 2} バイト、HEXMON へ戻った")
        return 0
    i = next((k for k in range(min(len(got), len(expect))) if got[k] != expect[k]), min(len(got), len(expect)))
    ln = expect.count("\n", 0, i)
    print(f"NG: {i} 文字目（{ln} 行目）で違う")
    print("想定:", repr(expect[max(0, i - 80):i + 60]))
    print("出力:", repr(got[max(0, i - 80):i + 60]))
    return 1

if __name__ == "__main__": sys.exit(main())
