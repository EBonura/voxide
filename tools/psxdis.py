#!/usr/bin/env python3
"""Minimal MIPS-I disassembler for PSX-EXE images.

PSoXide's PC sampler reports guest addresses and the linker leaves no symbols in
a PSX-EXE, so this is how a hot address becomes something readable.

Usage: psxdis.py <file.exe> <hex_start> [hex_end]
"""
import struct
import sys

R = ["zero","at","v0","v1","a0","a1","a2","a3","t0","t1","t2","t3","t4","t5","t6","t7",
     "s0","s1","s2","s3","s4","s5","s6","s7","t8","t9","k0","k1","gp","sp","fp","ra"]
SPECIAL = {0x00:"sll",0x02:"srl",0x03:"sra",0x04:"sllv",0x06:"srlv",0x07:"srav",
           0x08:"jr",0x09:"jalr",0x0c:"syscall",0x10:"mfhi",0x11:"mthi",0x12:"mflo",
           0x13:"mtlo",0x18:"mult",0x19:"multu",0x1a:"div",0x1b:"divu",0x20:"add",
           0x21:"addu",0x22:"sub",0x23:"subu",0x24:"and",0x25:"or",0x26:"xor",
           0x27:"nor",0x2a:"slt",0x2b:"sltu"}
OPS = {0x02:"j",0x03:"jal",0x04:"beq",0x05:"bne",0x06:"blez",0x07:"bgtz",0x08:"addi",
       0x09:"addiu",0x0a:"slti",0x0b:"sltiu",0x0c:"andi",0x0d:"ori",0x0e:"xori",
       0x0f:"lui",0x20:"lb",0x21:"lh",0x23:"lw",0x24:"lbu",0x25:"lhu",0x28:"sb",
       0x29:"sh",0x2b:"sw",0x32:"lwc2",0x3a:"swc2"}


def dis(w, pc):
    op = w >> 26
    rs, rt, rd = (w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31
    sa, imm = (w >> 6) & 31, w & 0xFFFF
    simm = imm - 0x10000 if imm & 0x8000 else imm
    if w == 0:
        return "nop"
    if op == 0:
        f = w & 0x3F
        n = SPECIAL.get(f, f"special?{f:02x}")
        if n in ("sll", "srl", "sra"):
            return f"{n} {R[rd]},{R[rt]},{sa}"
        if n in ("jr",):
            return f"jr {R[rs]}"
        if n in ("mult", "multu", "div", "divu"):
            return f"{n} {R[rs]},{R[rt]}"
        if n in ("mfhi", "mflo"):
            return f"{n} {R[rd]}"
        return f"{n} {R[rd]},{R[rs]},{R[rt]}"
    if op == 0x10:
        return f"cop0 0x{w & 0x1FFFFFF:07x}"
    if op == 0x12:  # COP2 / GTE
        if rs == 0:
            return f"mfc2 {R[rt]},cop2r{rd}"
        if rs == 2:
            return f"cfc2 {R[rt]},cop2c{rd}"
        if rs == 4:
            return f"mtc2 {R[rt]},cop2r{rd}"
        if rs == 6:
            return f"ctc2 {R[rt]},cop2c{rd}"
        return f"GTEop 0x{w & 0x1FFFFFF:07x}"
    n = OPS.get(op)
    if n is None:
        return f"?op{op:02x}"
    if n in ("j", "jal"):
        return f"{n} 0x{((w & 0x3FFFFFF) << 2) | (pc & 0xF0000000):08x}"
    if n in ("beq", "bne"):
        return f"{n} {R[rs]},{R[rt]},0x{pc + 4 + simm * 4:08x}"
    if n in ("blez", "bgtz"):
        return f"{n} {R[rs]},0x{pc + 4 + simm * 4:08x}"
    if n == "lui":
        return f"lui {R[rt]},0x{imm:x}"
    if op in (0x20, 0x21, 0x23, 0x24, 0x25, 0x28, 0x29, 0x2b, 0x32, 0x3a):
        reg = f"cop2r{rt}" if op in (0x32, 0x3a) else R[rt]
        return f"{n} {reg},{simm}({R[rs]})"
    return f"{n} {R[rt]},{R[rs]},{simm}"


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    data = open(sys.argv[1], "rb").read()
    base = struct.unpack("<I", data[0x10:0x14])[0]
    lo = int(sys.argv[2], 16)
    hi = int(sys.argv[3], 16) if len(sys.argv) > 3 else lo + 0x80
    for pc in range(lo, hi + 4, 4):
        off = 0x800 + (pc - base)
        if off + 4 > len(data):
            break
        w = struct.unpack("<I", data[off:off + 4])[0]
        print(f"{pc:08x}: {w:08x}  {dis(w, pc)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
