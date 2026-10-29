#!/usr/bin/env python3
"""W65C02S emulator with cycle counting for the real v2 machine (6502-CPLD board + HEXPAD, monitor HEXMON).

I/O (HEXMON programming guide 2.6):
  GPIO (6522 compatible) $8000-$800F : port A (LEDs PA7-4, switches PA3-0), timer T2 (one-shot), IFR.
                                       T1 ($8004-$8007) and ACR ($800B) belong to the monitor: touching them is an error.
  UART (6551 compatible) $8400-$8403 : tx_cycles = cycles the transmitter stays busy after each byte
                                       (0 = always ready; 19200bps at 1.8425MHz is about 960 cycles).
  HEXPAD $8800-$880F                 : HP_KEY (bit7 = new key, cleared on read), HP_CTRL, HP_HEX0-2, HP_DIGIT0-5.
  Monitor API $FF00-$FF11            : DISPLAY, POLL_KEY, WAIT_KEY, DELAY_MS, PUT_HEX, DELAY_10MS (emulated in Python).
The program is entered with JSR from the monitor (start(addr)); its final RTS returns to MONITOR_RET and stops the run.
Interrupts do not exist on this machine: CLI, WAI and BRK are errors. Writes to zero page $00-$1F are errors."""
from asm65 import OPS
DEC = {code: (m, mode, cyc, pen) for (m, mode), (code, cyc, pen) in OPS.items()}
GPIO, UART, HEXPAD = 0x8000, 0x8400, 0x8800
MONITOR_RET = 0xFFF0                     # 番兵：プログラムの最後の rts がここへ戻ったら終わり
SENTINEL = MONITOR_RET                   # compiler/tests/meteor_compare.py が使う名前
CLOCK = 1_842_500
API = {0xFF00: "DISPLAY", 0xFF03: "POLL_KEY", 0xFF06: "WAIT_KEY", 0xFF09: "DELAY_MS",
       0xFF0C: "PUT_HEX", 0xFF0F: "DELAY_10MS"}
SEG = ["0","1","2","3","4","5","6","7","8","9","A","b","C","d","E","F","-","H","=","J"," ","L","P","n","r","U","t","y","O","_","c","u"]

class Machine:
    def __init__(s, tx_cycles=0):
        s.mem = bytearray(0x10000)
        s.tx_cycles = tx_cycles; s.tx_busy_until = 0
        s.on_mark = None                 # 計測の目印（$7F00〜$7F04 への書き込み。cc65 版用）を受け取る関数
        s.on_wait_key = None             # WAIT_KEY で待つときに呼ばれる関数（キーを返す。None なら止める）
        s.A = s.X = s.Y = 0; s.S = 0xF8; s.P = 0x24; s.PC = 0   # モニタがスタックを少し使った状態
        s.cyc = 0; s.halted = False; s.s_min = s.S; s.s_start = s.S
        s.in_api = False; s.fast_wait = True
        # GPIO
        s.ora = 0; s.ddra = 0; s.switches = 0x0F
        s.t2_latch_lo = 0; s.t2_fire = None; s.ifr = 0
        # UART
        s.rx = None; s.out = bytearray(); s.uart_cmd = None
        # HEXPAD
        s.key_new = False; s.key_code = 0; s.hp_ctrl = 0x8A; s.digits = [16] * 6
        s.digit_log = []
    def start(s, addr):
        """モニタが JSR addr で呼んだ状態にする"""
        r = MONITOR_RET - 1; s.push(r >> 8); s.push(r); s.s_start = s.S; s.s_min = s.S; s.PC = addr
    def press(s, code):
        s.key_new = True; s.key_code = code & 0x1F
    def display(s):
        return "".join(SEG[d & 31] for d in reversed(s.digits))
    # ---- memory ----
    def t2(s):
        if s.t2_fire is not None and s.cyc >= s.t2_fire: s.ifr |= 0x20; s.t2_fire = None
    def rd(s, a):
        if GPIO <= a < GPIO + 16:
            r = a - GPIO; s.t2()
            if r in (1, 15): return (s.ora & s.ddra) | (s.switches & ~s.ddra & 0xFF)
            if r == 3: return s.ddra
            if r == 8: s.ifr &= ~0x20; return 0
            if r == 13:
                if not s.ifr & 0x20 and s.t2_fire is not None and s.fast_wait:
                    s.cyc = max(s.cyc, s.t2_fire); s.t2()   # T2 を待つ間は時間を一気に進める（フレームの計測の外）
                return s.ifr
            if r in (4, 5, 6, 7, 11): raise RuntimeError(f"monitor's T1/ACR read at {a:04X}")
            return 0
        if UART <= a < UART + 4:
            r = a - UART
            if r == 0: c = s.rx or 0; s.rx = None; return c
            if r == 1: return (0x10 if s.cyc >= s.tx_busy_until else 0) | (0x08 if s.rx is not None else 0)
            if r == 2: return s.uart_cmd or 0
            return 0x1F
        if HEXPAD <= a < HEXPAD + 16:
            r = a - HEXPAD
            if r == 0:
                v = (0x80 if s.key_new else 0) | s.key_code; s.key_new = False; return v
            if r == 2: return s.hp_ctrl
            if 4 <= r <= 6: i = (r - 4) * 2; return ((s.digits[i + 1] & 15) << 4) | (s.digits[i] & 15)
            if 8 <= r <= 13: return s.digits[r - 8]
            if r == 15: return 1
            return 0
        return s.mem[a]
    def wr(s, a, v):
        v &= 0xFF
        if a < 0x20 and not s.in_api: raise RuntimeError(f"write to monitor zero page ${a:02X} at {s.PC:04X}")
        if GPIO <= a < GPIO + 16:
            r = a - GPIO; s.t2()
            if r in (1, 15): s.ora = v
            elif r == 3: s.ddra = v
            elif r == 8: s.t2_latch_lo = v
            elif r == 9: s.t2_fire = s.cyc + (s.t2_latch_lo | (v << 8)) + 2   # フラグは T2CL を読んだときだけ消す（ガイドどおり）
            elif r == 13: s.ifr &= ~(v & 0x7F)
            elif r in (4, 5, 6, 7, 11): raise RuntimeError(f"monitor's T1/ACR written at {a:04X}")
            return
        if UART <= a < UART + 4:
            r = a - UART
            if r == 0:
                if s.uart_cmd is None: raise RuntimeError("UART used before the command register was set")
                s.out.append(v); s.tx_busy_until = s.cyc + s.tx_cycles
            elif r == 2: s.uart_cmd = v
            return
        if HEXPAD <= a < HEXPAD + 16:
            r = a - HEXPAD
            if r == 2: s.hp_ctrl = v
            elif 4 <= r <= 6: i = (r - 4) * 2; s.digits[i] = v & 15; s.digits[i + 1] = v >> 4
            elif 8 <= r <= 13: s.digits[r - 8] = v & 31
            if 4 <= r <= 13: s.digit_log.append((s.cyc, s.display()))
            return
        if 0x7F00 <= a <= 0x7F04:
            if s.on_mark: s.on_mark(a - 0x7F00)
            return
        s.mem[a] = v
    def w16(s, a): return s.mem[a] | (s.mem[(a + 1) & 0xFFFF] << 8)
    def push(s, v):
        s.mem[0x100 + s.S] = v & 0xFF; s.S = (s.S - 1) & 0xFF; s.s_min = min(s.s_min, s.S)
    def pull(s): s.S = (s.S + 1) & 0xFF; return s.mem[0x100 + s.S]
    def nz(s, v):
        v &= 0xFF; s.P = (s.P & 0x7D) | (v & 0x80) | (0x02 if v == 0 else 0); return v
    def fl(s, bit, on): s.P = (s.P | bit) if on else (s.P & ~bit)
    def api(s, name):
        """モニタ API を Python で行う（サイクル数はおおよそ）"""
        s.in_api = True
        if s.P & 0x08: raise RuntimeError(f"{name} called in decimal mode")
        if name == "DISPLAY":
            for i in range(6): s.digits[i] = s.mem[2 + i] & 31
            s.digit_log.append((s.cyc, s.display())); s.cyc += 80
        elif name == "POLL_KEY":
            v = s.rd(HEXPAD); s.cyc += 20
            if v & 0x80: s.A = v & 0x1F; s.P |= 1
            else: s.P &= ~1
        elif name == "WAIT_KEY":
            v = s.rd(HEXPAD)
            if not v & 0x80:
                k = s.on_wait_key(s) if s.on_wait_key else None
                if k is None: raise RuntimeError("WAIT_KEY with no key to give")
                s.cyc += 20000; v = 0x80 | k
            s.A = v & 0x1F; s.cyc += 20
        elif name in ("DELAY_MS", "DELAY_10MS"):
            n = s.A or 256; ms = n * (10 if name == "DELAY_10MS" else 1)
            s.cyc += ms * (CLOCK // 1000) + 30
            if name == "DELAY_10MS": s.X = 0
        elif name == "PUT_HEX":
            s.mem[(2 + s.X) & 0xFF] = s.A & 15; s.mem[(3 + s.X) & 0xFF] = s.A >> 4; s.cyc += 40
        s.in_api = False
        s.PC = ((s.pull() | (s.pull() << 8)) + 1) & 0xFFFF; s.cyc += 6
    # ---- execution ----
    def step(s):
        if s.PC == MONITOR_RET: s.halted = True; return
        if s.PC in API: s.api(API[s.PC]); return
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
        elif m in ("clc", "sec", "sei", "cld", "sed", "clv"):
            bit = {"clc": 1, "sec": 1, "sei": 4, "cld": 8, "sed": 8, "clv": 0x40}[m]
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
        elif m in ("rti", "wai", "cli"): raise RuntimeError(f"{m} at {pc:04X}: this machine has no interrupts")
        elif m == "stp": s.halted = True
        elif m == "nop": pass
        elif m == "brk": raise RuntimeError(f"BRK at {pc:04X}")
        else: raise RuntimeError("unimplemented " + m)
        s.cyc += cyc + extra
