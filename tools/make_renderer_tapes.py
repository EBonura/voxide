#!/usr/bin/env python3
"""Generate the synthetic standing and walking/mining renderer benchmarks.

PXITAPE1 samples are tied to emulated video time. These normal controller
routes measure shipping cadence; they are not an exact simulation-state oracle
when two builds render at different rates. Start from a cold disc boot.
"""
import argparse
import struct
from pathlib import Path


def tape(actions):
    frames = 3500
    data = bytearray(b'PXITAPE1' + struct.pack('<I', frames))
    pulses = [(1360, 0x100), (1400, 0x800), (1440, 0x100),
              (1480, 0x400), (1520, 0x100), (1900, 0x100),
              (1940, 0x800), (1980, 0x100), (2020, 0x400),
              (2060, 0x100), (2120, 0x8000), (2240, 0x8000)]
    for tick in range(frames):
        buttons = 8 if 700 <= tick < 760 else 0
        rx = ry = lx = ly = 128
        if actions:
            if 900 <= tick < 1320:
                ly = 28
            if 1260 <= tick < 1320:
                ry = 188
            for start, button in pulses:
                if start <= tick < start + 10:
                    buttons |= button
            if 1560 <= tick < 1860:
                buttons |= 0x200
            if 2380 <= tick < 2560:
                rx = 213
        data.extend(struct.pack('<H4B', buttons, rx, ry, lx, ly))
    return bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    for name, actions in [('standing', False), ('actions', True)]:
        path = args.out / (name + '.pxtape')
        data = tape(actions)
        if path.exists() and path.read_bytes() != data:
            raise FileExistsError(f'refusing to replace different tape: {path}')
        path.write_bytes(data)
        print(path)


if __name__ == '__main__':
    main()
