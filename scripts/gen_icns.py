#!/usr/bin/env python3
"""生成 macOS 应用图标 assets/AppIcon.icns。

菜单栏图标由 src/icons.rs 在编译期生成（32×32 模板图，只能单色）；应用图标需要
完整彩色图，因此这里用同一套「键盘」语汇重新画一份高分辨率版本。

只依赖 Python 标准库：像素用纯 Python 渲染成 PNG，再交给 iconutil 打包。
产物已提交进仓库，CI 不需要运行本脚本；改了图形再重跑即可：

    python3 -I scripts/gen_icns.py
"""

from __future__ import annotations

import math
import os
import struct
import subprocess
import sys
import tempfile
import zlib

CANVAS = 1024
SUBSAMPLES = 4

# macOS 应用图标规范：1024 画布里图形占 824×824，圆角半径 185。
MARGIN = (CANVAS - 824) // 2
CONTENT = 824
BG_RADIUS = 185

# 背景近似线性渐变：上靛蓝、下紫罗兰。
BG_TOP = (0x4F, 0x46, 0xE5)
BG_BOTTOM = (0x7C, 0x3A, 0xED)
FG = (0xFF, 0xFF, 0xFF)


class Rect:
    """轴对齐圆角矩形。"""

    __slots__ = ("left", "top", "right", "bottom", "radius")

    def __init__(self, left: float, top: float, right: float, bottom: float, radius: float):
        self.left = left
        self.top = top
        self.right = right
        self.bottom = bottom
        self.radius = radius

    def span_at(self, y: float) -> tuple[float, float] | None:
        """返回该水平线被矩形覆盖的 x 区间；未覆盖则为 None。"""
        if y < self.top or y > self.bottom:
            return None
        # 距离上下边缘最近的一条：只有进入圆角区域才需要横向内缩。
        dy = min(y - self.top, self.bottom - y)
        r = self.radius
        if dy >= r:
            return (self.left, self.right)
        dx = r - math.sqrt(max(r * r - (r - dy) * (r - dy), 0.0))
        return (self.left + dx, self.right - dx)


def keyboard_shapes() -> tuple[Rect, list[Rect]]:
    """返回键盘主体与按键镂空。尺寸按 1024 画布的内容区比例推导。"""
    w = CONTENT * 0.74
    h = w / 1.55
    left = MARGIN + (CONTENT - w) / 2
    top = MARGIN + (CONTENT - h) / 2
    body = Rect(left, top, left + w, top + h, h * 0.14)

    pad = h * 0.13
    gap = h * 0.075
    key = (h - 2 * pad - 3 * gap) / 4.0  # 4 行等高（含空格行）
    radius = key * 0.28
    inner_left = left + pad
    inner_right = left + w - pad
    inner_width = inner_right - inner_left

    cuts: list[Rect] = []

    def row(y: float) -> float:
        return top + pad + y * (key + gap)

    # 前三行：每行 4 个等宽键。
    col_gap = gap * 0.9
    key_w = (inner_width - 3 * col_gap) / 4.0
    for r in range(3):
        y = row(r)
        for c in range(4):
            x = inner_left + c * (key_w + col_gap)
            cuts.append(Rect(x, y, x + key_w, y + key, radius))

    # 第四行：两个小键 + 空格条。
    y = row(3)
    side_w = key_w
    cuts.append(Rect(inner_left, y, inner_left + side_w, y + key, radius))
    cuts.append(Rect(inner_right - side_w, y, inner_right, y + key, radius))
    space_left = inner_left + side_w + col_gap
    space_right = inner_right - side_w - col_gap
    cuts.append(Rect(space_left, y, space_right, y + key, radius * 0.8))

    return body, cuts


def add_span(row: list[float], x0: float, x1: float, weight: float) -> None:
    """把 [x0, x1) 区间按覆盖率累加进一行像素（端点做精确的分数覆盖）。"""
    x0 = max(x0, 0.0)
    x1 = min(x1, float(CANVAS))
    if x1 <= x0:
        return
    i0 = int(x0)
    i1 = int(x1)
    if i0 == i1:
        row[i0] += (x1 - x0) * weight
        return
    row[i0] += (i0 + 1 - x0) * weight
    for i in range(i0 + 1, i1):
        row[i] += weight
    if i1 < CANVAS:
        row[i1] += (x1 - i1) * weight


def render() -> bytes:
    body, cuts = keyboard_shapes()
    bg = Rect(MARGIN, MARGIN, MARGIN + CONTENT, MARGIN + CONTENT, BG_RADIUS)

    rgba = bytearray(CANVAS * CANVAS * 4)
    inv = 1.0 / SUBSAMPLES

    for y in range(CANVAS):
        alpha_row = [0.0] * CANVAS
        fg_row = [0.0] * CANVAS

        for s in range(SUBSAMPLES):
            sy = y + (s + 0.5) * inv

            bg_span = bg.span_at(sy)
            if bg_span is None:
                continue
            add_span(alpha_row, bg_span[0], bg_span[1], inv)

            body_span = body.span_at(sy)
            if body_span is None:
                continue
            # 主体覆盖 = 主体区间 − 各按键镂空区间（按键互不重叠）。
            covered = [body_span]
            for cut in cuts:
                cut_span = cut.span_at(sy)
                if cut_span is None:
                    continue
                next_covered = []
                for lo, hi in covered:
                    if cut_span[1] <= lo or cut_span[0] >= hi:
                        next_covered.append((lo, hi))
                        continue
                    if cut_span[0] > lo:
                        next_covered.append((lo, cut_span[0]))
                    if cut_span[1] < hi:
                        next_covered.append((cut_span[1], hi))
                covered = next_covered
                if not covered:
                    break
            for lo, hi in covered:
                add_span(fg_row, lo, hi, inv)

        # 渐变按行插值（图形内部为纯白）。
        t = (y - MARGIN) / CONTENT
        t = min(max(t, 0.0), 1.0)
        bg_r = BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t
        bg_g = BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t
        bg_b = BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t

        base = y * CANVAS * 4
        for x in range(CANVAS):
            a = alpha_row[x]
            if a <= 0.0:
                continue
            f = fg_row[x]
            idx = base + x * 4
            rgba[idx] = int(bg_r + (FG[0] - bg_r) * f + 0.5)
            rgba[idx + 1] = int(bg_g + (FG[1] - bg_g) * f + 0.5)
            rgba[idx + 2] = int(bg_b + (FG[2] - bg_b) * f + 0.5)
            rgba[idx + 3] = int(a * 255 + 0.5)

    return bytes(rgba)


def png_bytes(rgba: bytes, size: int) -> bytes:
    """把整张 CANVAS 图按面积平均缩放到 size×size 并编码为 PNG。"""
    factor = CANVAS // size
    rows = bytearray()
    for y in range(size):
        rows.append(0)  # 每行的 filter type 为 None
        for x in range(size):
            r = g = b = a = 0
            for dy in range(factor):
                base = ((y * factor + dy) * CANVAS + x * factor) * 4
                for dx in range(factor):
                    idx = base + dx * 4
                    # 预乘 alpha 再平均，避免透明边缘出现暗边。
                    alpha = rgba[idx + 3]
                    r += rgba[idx] * alpha
                    g += rgba[idx + 1] * alpha
                    b += rgba[idx + 2] * alpha
                    a += alpha
            n = factor * factor
            if a:
                rows.append(min(r // a, 255))
                rows.append(min(g // a, 255))
                rows.append(min(b // a, 255))
                rows.append(min(a // n, 255))
            else:
                rows.extend((0, 0, 0, 0))

    def chunk(tag: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + tag
            + payload
            + struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(rows), 9))
        + chunk(b"IEND", b"")
    )


def main() -> int:
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out = os.path.join(root, "assets", "AppIcon.icns")
    os.makedirs(os.path.dirname(out), exist_ok=True)

    print("渲染 1024×1024 …", file=sys.stderr)
    pixels = render()

    with tempfile.TemporaryDirectory() as tmp:
        iconset = os.path.join(tmp, "AppIcon.iconset")
        os.makedirs(iconset)
        for size in (16, 32, 128, 256, 512):
            for scale, suffix in ((1, ""), (2, "@2x")):
                px = size * scale
                name = f"icon_{size}x{size}{suffix}.png"
                with open(os.path.join(iconset, name), "wb") as fh:
                    fh.write(png_bytes(pixels, px))
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", out], check=True)

    print(f"已写入 {out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())