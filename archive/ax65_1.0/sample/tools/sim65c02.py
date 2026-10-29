#!/usr/bin/env python3
"""Small W65C02S emulator with cycle counting, a VIA (6522) timer 1 and an ACIA (6551).

tx_cycles: cycles the ACIA stays busy after each transmitted byte (0 = always ready).
At 1 MHz, 9600 bps (10 bits/char) is about 1042 cycles."""
from asm65 import OPS
DEC = {code: (m, mode, cyc, pen) for (m, mode), (code, cyc, pen) in OPS.items()}
VIA, ACIA, MONITOR = 0x6000, 0x8000, 0xFF1F

class Machine:
    def __init__(s, tx_cycles=0):
        s.mem = bytearray(0x10000)
        s.tx_cycles = tx_cycles; s.tx_busy_until = 0
        s.on_mark = None                 # 計測の目印（$7000〜$7004 への書き込み）を受け取る関数
        s.A = s.X = s.Y = 0; s.S = 0xFD; s.P = 0x24; s.PC = 0
        s.cyc = 0; s.halted = False; s.waiting = False
        # VIA
        s.latch = 0; s.ifr = 0; s.ier = 0; s.next_fire = None
        # ACIA
        s.rx = None; s.out = bytearray()
        # ROM: IRQ stub "jmp ($0280)" at $FF00, vectors
        s.mem[0xFF00:0xFF03] = bytes([0x6C, 0x80, 0x02])
        s.mem[0xFFFE] = 0x00; s.mem[0xFFFF] = 0xFF
        s.irq_count = 0; s.in_irq = False; s.irq_start = 0; s.irq_cycles = 0; s.irq_max = 0
    # ---- memory ----
    def rd(s, a):
        if VIA <= a < VIA + 16:
            r = a - VIA
            if r == 4: s.ifr &= ~0x40; return 0
            if r == 13: return s.ifr | (0x80 if s.ifr & s.ier & 0x7F else 0)
            if r == 14: return s.ier | 0x80
            return 0
        if a == ACIA:
            c = s.rx or 0; s.rx = None; return c
        if a == ACIA + 1:
            ready = 0x10 if s.cyc >= s.tx_busy_until else 0
            return ready | (0x08 if s.rx is not None else 0)
        return s.mem[a]
    def wr(s, a, v):
        v &= 0xFF
        if VIA <= a < VIA + 16:
            r = a - VIA
            if r == 4: s.latch = (s.latch & 0xFF00) | v
            elif r == 5:
                s.latch = (s.latch & 0xFF) | (v << 8); s.ifr &= ~0x40; s.next_fire = s.cyc + s.latch + 1
            elif r == 13: s.ifr &= ~v
            elif r == 14: s.ier = (s.ier | (v & 0x7F)) if v & 0x80 else (s.ier & ~v)
            return
        if a == ACIA: s.out.append(v); s.tx_busy_until = s.cyc + s.tx_cycles; return
        if 0x7000 <= a <= 0x7004:
            if s.on_mark: s.on_mark(a - 0x7000)
            return
        if a == ACIA + 1: return
        s.mem[a] = v
    def w16(s, a): return s.mem[a] | (s.mem[(a + 1) & 0xFFFF] << 8)
    def push(s, v): s.mem[0x100 + s.S] = v & 0xFF; s.S = (s.S - 1) & 0xFF
    def pull(s): s.S = (s.S + 1) & 0xFF; return s.mem[0x100 + s.S]
    def nz(s, v):
        v &= 0xFF; s.P = (s.P & 0x7D) | (v & 0x80) | (0x02 if v == 0 else 0); return v
    def fl(s, bit, on): s.P = (s.P | bit) if on else (s.P & ~bit)
    def irq_line(s): return bool(s.ifr & s.ier & 0x7F)
    def timers(s):
        while s.next_fire is not None and s.cyc >= s.next_fire:
            s.ifr |= 0x40; s.next_fire += s.latch + 2
    # ---- execution ----
    def step(s):
        s.timers()
        if s.waiting:
            if not s.irq_line():
                s.cyc = max(s.cyc, s.next_fire); s.timers()
            s.waiting = False
        if s.irq_line() and not (s.P & 0x04):
            s.irq_start = s.cyc; s.in_irq = True; s.irq_count += 1
            s.push(s.PC >> 8); s.push(s.PC); s.push((s.P | 0x20) & ~0x10)
            s.P = (s.P | 0x04) & ~0x08; s.PC = s.w16(0xFFFE); s.cyc += 7
            return
        pc = s.PC; opc = s.mem[pc]
        if opc not in DEC: raise RuntimeError(f"bad opcode {opc:02X} at {pc:04X}")
        m, mode, cyc, pen = DEC[opc]
        b1 = s.mem[(pc + 1) & 0xFFFF]; w = s.w16((pc + 1) & 0xFFFF)
        size = {"imp":1,"acc":1,"imm":2,"zp":2,"zpx":2,"zpy":2,"izx":2,"izy":2,"izp":2,"rel":2}.get(mode, 3)
        s.PC = (pc + size) & 0xFFFF
        extra = 0; ea = None
        if mode == "zp": ea = b1
        elif mode == "zpx": ea = (b1 + s.X) & 0xFF
        elif mode == "zpy": ea = (b1 + s.Y) & 0xFF
        elif mode == "abs": ea = w
        elif mode in ("abx", "aby"):
            base = w; ea = (base + (s.X if mode == "abx" else s.Y)) & 0xFFFF
            if pen and (ea & 0xFF00) != (base & 0xFF00): extra += 1
        elif mode == "izx": z = (b1 + s.X) & 0xFF; ea = s.mem[z] | (s.mem[(z + 1) & 0xFF] << 8)
        elif mode == "izy":
            base = s.mem[b1] | (s.mem[(b1 + 1) & 0xFF] << 8); ea = (base + s.Y) & 0xFFFF
            if pen and (ea & 0xFF00) != (base & 0xFF00): extra += 1
        elif mode == "izp": ea = s.mem[b1] | (s.mem[(b1 + 1) & 0xFF] << 8)
        elif mode == "ind": ea = s.w16(w)
        elif mode == "iax": ea = s.w16((w + s.X) & 0xFFFF)
        def val():
            if mode == "imm": return b1
            if mode == "acc": return s.A
            return s.rd(ea)
        def store(v):
            if mode == "acc": s.A = v & 0xFF
            else: s.wr(ea, v)
        C = s.P & 1
        if m == "lda": s.A = s.nz(val())
        elif m == "ldx": s.X = s.nz(val())
        elif m == "ldy": s.Y = s.nz(val())
        elif m == "sta": s.wr(ea, s.A)
        elif m == "stx": s.wr(ea, s.X)
        elif m == "sty": s.wr(ea, s.Y)
        elif m == "stz": s.wr(ea, 0)
        elif m in ("adc", "sbc"):
            v = val()
            if m == "sbc" and not (s.P & 0x08): v ^= 0xFF
            if s.P & 0x08:
                extra += 1
                if m == "adc":
                    lo = (s.A & 15) + (v & 15) + C
                    if lo > 9: lo += 6
                    hi = (s.A >> 4) + (v >> 4) + (1 if lo > 15 else 0)
                    if hi > 9: hi += 6
                    r = ((hi << 4) | (lo & 15)) & 0xFF; s.fl(1, hi > 15)
                else:
                    lo = (s.A & 15) - (v & 15) - (1 - C); hi = (s.A >> 4) - (v >> 4)
                    if lo < 0: lo -= 6; hi -= 1
                    if hi < 0: hi -= 6
                    r = ((hi << 4) | (lo & 15)) & 0xFF; s.fl(1, s.A - v - (1 - C) >= 0)
                s.A = s.nz(r)
            else:
                t = s.A + v + C; r = t & 0xFF
                s.fl(0x40, (~(s.A ^ v) & (s.A ^ r) & 0x80) != 0); s.fl(1, t > 0xFF); s.A = s.nz(r)
        elif m in ("and", "ora", "eor"):
            v = val(); s.A = s.nz(s.A & v if m == "and" else s.A | v if m == "ora" else s.A ^ v)
        elif m in ("cmp", "cpx", "cpy"):
            r = {"cmp": s.A, "cpx": s.X, "cpy": s.Y}[m]; v = val(); s.fl(1, r >= v); s.nz(r - v)
        elif m == "bit":
            v = val(); s.fl(2, (s.A & v) == 0)
            if mode != "imm": s.fl(0x80, v & 0x80); s.fl(0x40, v & 0x40)
        elif m in ("tsb", "trb"):
            v = s.rd(ea); s.fl(2, (s.A & v) == 0); s.wr(ea, v | s.A if m == "tsb" else v & ~s.A)
        elif m in ("asl", "lsr", "rol", "ror"):
            v = val()
            if m == "asl": s.fl(1, v & 0x80); r = v << 1
            elif m == "lsr": s.fl(1, v & 1); r = v >> 1
            elif m == "rol": s.fl(1, v & 0x80); r = (v << 1) | C
            else: s.fl(1, v & 1); r = (v >> 1) | (C << 7)
            store(s.nz(r))
        elif m in ("inc", "dec"): store(s.nz(val() + (1 if m == "inc" else -1)))
        elif m == "inx": s.X = s.nz(s.X + 1)
        elif m == "iny": s.Y = s.nz(s.Y + 1)
        elif m == "dex": s.X = s.nz(s.X - 1)
        elif m == "dey": s.Y = s.nz(s.Y - 1)
        elif m == "tax": s.X = s.nz(s.A)
        elif m == "tay": s.Y = s.nz(s.A)
        elif m == "txa": s.A = s.nz(s.X)
        elif m == "tya": s.A = s.nz(s.Y)
        elif m == "tsx": s.X = s.nz(s.S)
        elif m == "txs": s.S = s.X
        elif m == "pha": s.push(s.A)
        elif m == "phx": s.push(s.X)
        elif m == "phy": s.push(s.Y)
        elif m == "php": s.push(s.P | 0x30)
        elif m == "pla": s.A = s.nz(s.pull())
        elif m == "plx": s.X = s.nz(s.pull())
        elif m == "ply": s.Y = s.nz(s.pull())
        elif m == "plp": s.P = (s.pull() | 0x20) & ~0x10
        elif m in ("clc", "sec", "cli", "sei", "cld", "sed", "clv"):
            bit = {"clc": 1, "sec": 1, "cli": 4, "sei": 4, "cld": 8, "sed": 8, "clv": 0x40}[m]
            s.fl(bit, m in ("sec", "sei", "sed"))
        elif mode == "rel":
            cond = {"bpl": not s.P & 0x80, "bmi": s.P & 0x80, "bvc": not s.P & 0x40, "bvs": s.P & 0x40,
                    "bcc": not s.P & 1, "bcs": s.P & 1, "bne": not s.P & 2, "beq": s.P & 2, "bra": True}[m]
            if cond:
                tgt = (s.PC + (b1 - 256 if b1 > 127 else b1)) & 0xFFFF
                if m != "bra": extra += 1
                if (tgt & 0xFF00) != (s.PC & 0xFF00): extra += 1
                s.PC = tgt
        elif m == "jmp": s.PC = ea
        elif m == "jsr": r = (pc + 2) & 0xFFFF; s.push(r >> 8); s.push(r); s.PC = ea
        elif m == "rts": s.PC = ((s.pull() | (s.pull() << 8)) + 1) & 0xFFFF
        elif m == "rti":
            s.P = (s.pull() | 0x20) & ~0x10; s.PC = s.pull() | (s.pull() << 8)
            if s.in_irq:
                s.in_irq = False; c = s.cyc + cyc - s.irq_start
                s.irq_cycles += c; s.irq_max = max(s.irq_max, c)
        elif m == "wai": s.waiting = True
        elif m == "stp": s.halted = True
        elif m == "nop": pass
        elif m == "brk": raise RuntimeError(f"BRK at {pc:04X}")
        else: raise RuntimeError("unimplemented " + m)
        s.cyc += cyc + extra
