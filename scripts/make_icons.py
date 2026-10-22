"""Genera los iconos de WiZ Tray (foquito ámbar) sin dependencias externas.

Uso: python scripts/make_icons.py
Salida: src-tauri/icons/icon.ico (16/32/48) y src-tauri/icons/tray.ico (16/32).
"""

import math
import os
import struct

GLASS = (240, 178, 96)
GLASS_CORE = (255, 226, 176)
METAL = (168, 170, 182)

OUT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "icons")


def _coverage(px, size, x, y, shape):
    """Muestreo 3x3 para suavizar los bordes."""
    hits = 0
    for sy in range(3):
        for sx in range(3):
            fx = (x + (sx + 0.5) / 3) / size
            fy = (y + (sy + 0.5) / 3) / size
            if shape(fx, fy):
                hits += 1
    return hits / 9.0


def _bulb_shape(fx, fy):
    """Devuelve el color en (fx, fy) normalizado o None."""
    gx, gy, gr = 0.5, 0.44, 0.30
    if (fx - gx) ** 2 + (fy - gy) ** 2 <= gr * gr:
        return GLASS
    if 0.41 <= fx <= 0.59 and 0.70 <= fy <= 0.88:
        return METAL
    if 0.44 <= fx <= 0.56 and 0.86 <= fy <= 0.94:
        return METAL
    return None


def render(size):
    rows = []
    for y in range(size):
        row = bytearray()
        for x in range(size):
            acc = [0.0, 0.0, 0.0]
            alpha = 0.0
            for sy in range(4):
                for sx in range(4):
                    fx = (x + (sx + 0.5) / 4) / size
                    fy = (y + (sy + 0.5) / 4) / size
                    color = _bulb_shape(fx, fy)
                    if color is None:
                        continue
                    a = 1.0
                    if color is GLASS:
                        core = (fx - 0.5) ** 2 + (fy - 0.42) ** 2 <= 0.06**2
                        if core:
                            color = GLASS_CORE
                    for i in range(3):
                        acc[i] += color[i] * a
                    alpha += a
            if alpha <= 0:
                row += bytes((0, 0, 0, 0))
            else:
                px = [int(acc[i] / alpha) for i in range(3)]
                row += bytes((px[0], px[1], px[2], int(round(255 * alpha / 16))))
        rows.append(bytes(row))
    return rows


def bmp_image(size):
    rows = render(size)
    header = struct.pack(
        "<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0
    )
    pixels = b"".join(rows[::-1])
    mask = b"\x00" * (size * 4)
    return header + pixels + mask


def write_ico(path, sizes):
    images = [(size, bmp_image(size)) for size in sizes]
    offset = 6 + 16 * len(images)
    entries = b""
    for size, data in images:
        entries += struct.pack(
            "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset
        )
        offset += len(data)
    with open(path, "wb") as handle:
        handle.write(struct.pack("<HHH", 0, 1, len(images)))
        handle.write(entries)
        for _, data in images:
            handle.write(data)


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    write_ico(os.path.join(OUT_DIR, "icon.ico"), [16, 32, 48])
    write_ico(os.path.join(OUT_DIR, "tray.ico"), [16, 32])
    print("iconos escritos en", os.path.normpath(OUT_DIR))


if __name__ == "__main__":
    main()
