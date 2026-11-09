#!/usr/bin/env python3
"""Remove Inter's private-use icon range from the bundled UI fonts.

Inter 3.19 ships a set of glyphs in the Unicode private use area (U+E000 to
U+F8FF). Those glyphs shadow Phosphor icons whenever Inter is first in egui's
font fallback chain, so `icons::CARET_DOWN` and friends render as Inter's
private glyphs instead of the Phosphor glyphs.

This script deletes the private-use cmap entries. The glyph outlines stay in
the font (so OpenType features that reference them keep working), but text
layout will fall through to Phosphor for those codepoints.

Usage:
    python3 scripts/strip-inter-pua.py assets/fonts/Inter-Regular.ttf \
        assets/fonts/Inter-SemiBold.ttf
"""

from __future__ import annotations

import sys

from fontTools.ttLib import TTFont


def strip_private_use(path: str) -> int:
    font = TTFont(path)
    removed = 0
    for table in font["cmap"].tables:
        for codepoint in [cp for cp in table.cmap if 0xE000 <= cp <= 0xF8FF]:
            del table.cmap[codepoint]
            removed += 1
    if removed:
        font.save(path)
    return removed


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    for path in argv[1:]:
        removed = strip_private_use(path)
        print(f"{path}: removed {removed} private-use cmap entries")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
