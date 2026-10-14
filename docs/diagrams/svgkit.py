"""Small helpers for the hand-laid-out architecture diagrams.

Every diagram is drawn for the docs content column (about 690px wide), so text
keeps its size on the page. Colors come from CSS variables with a light and a
dark set; the SVG follows the reader's color scheme on GitHub and on the site.
"""
from __future__ import annotations

import html
import re

STYLE = """
svg { --bg: #fbfbfd; --band: #f2f1fb; --band-line: #dcd8f4; --node: #ffffff; --node-line: #b8b1ea;
  --ext: #f6f6f8; --ext-line: #c9c9d6; --dec: #fff8ec; --dec-line: #e2b766; --end: #eef6f1; --end-line: #8cc3a2;
  --stop: #fbefef; --stop-line: #e0a3a3; --title: #1f1f2e; --sub: #5d5d72; --label: #6e6a92; --line: #77778d;
  --accent: #7c6fe0; --life: #c7c4dc; --block: #8c88b0; }
@media (prefers-color-scheme: dark) {
  svg { --bg: #1c1c23; --band: #252432; --band-line: #3b3854; --node: #2d2c3c; --node-line: #5e57a8;
    --ext: #262630; --ext-line: #4a4a5e; --dec: #332c1f; --dec-line: #a3813f; --end: #1f2e26; --end-line: #4f8a66;
    --stop: #352223; --stop-line: #9a5c5c; --title: #ececf4; --sub: #a9a9bb; --label: #a39ee0; --line: #9a9ab0;
    --accent: #a99ff0; --life: #47455e; --block: #8f8ab8; }
}
text { font-family: system-ui, -apple-system, "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", sans-serif; }
.bg { fill: var(--bg); }
.band { fill: var(--band); stroke: var(--band-line); }
.band-label { fill: var(--label); font-size: 13px; font-weight: 600; }
.node { fill: var(--node); stroke: var(--node-line); stroke-width: 1.2; }
.ext { fill: var(--ext); stroke: var(--ext-line); stroke-dasharray: 4 3; }
.dec { fill: var(--dec); stroke: var(--dec-line); stroke-width: 1.2; }
.end { fill: var(--end); stroke: var(--end-line); stroke-width: 1.2; }
.stop { fill: var(--stop); stroke: var(--stop-line); stroke-width: 1.2; }
.t { fill: var(--title); font-size: 14px; font-weight: 600; text-anchor: middle; }
.s { fill: var(--sub); font-size: 12px; text-anchor: middle; }
.edge { fill: none; stroke: var(--line); stroke-width: 1.4; }
.back { fill: none; stroke: var(--accent); stroke-width: 1.4; stroke-dasharray: 5 4; }
.el { fill: var(--sub); font-size: 11.5px; paint-order: stroke; stroke: var(--bg); stroke-width: 4px; stroke-linejoin: round; }
.el-band { stroke: var(--band); }
.bl { fill: var(--accent); }
.head { fill: var(--line); }
.head-back { fill: var(--accent); }
.life { stroke: var(--life); stroke-width: 1.2; stroke-dasharray: 4 4; }
.block { fill: none; stroke: var(--block); stroke-width: 1; }
.block-tag { fill: var(--block); }
.block-tag-text { fill: var(--bg); font-size: 11px; font-weight: 700; }
.block-label { fill: var(--label); font-size: 11.5px; paint-order: stroke; stroke: var(--bg); stroke-width: 4px; stroke-linejoin: round; }
.block-split { stroke: var(--block); stroke-width: 1; stroke-dasharray: 5 4; }
.note { fill: var(--dec); stroke: var(--dec-line); }
"""

DEFS = """<defs>
  <marker id="a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path class="head" d="M0 0L10 5L0 10z"/></marker>
  <marker id="b" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path class="head-back" d="M0 0L10 5L0 10z"/></marker>
</defs>"""


def text_width(text: str, size: float, bold: bool = False) -> float:
    """Rough rendered width: CJK and full-width characters are one em, the rest a bit over half."""
    width = 0.0
    for ch in text:
        if ord(ch) >= 0x2E80:
            width += size
        elif ch in 'il.,:;|!\'` ':
            width += size * 0.32
        elif ch.isupper() or ch in 'mwMW_':
            width += size * 0.62
        else:
            width += size * 0.56
    return width * (1.04 if bold else 1.0)


def wrap(text: str, size: float, max_width: float, bold: bool = False) -> list[str]:
    """Wraps at CJK characters and between Latin words; an explicit \\n always breaks.

    Lines are balanced: the width shrinks as long as the line count stays the
    same, so a paragraph never ends with one stray character.
    """
    lines = _greedy(text, size, max_width, bold)
    lo, hi = max_width * 0.4, max_width
    while hi - lo > 2:
        mid = (lo + hi) / 2
        if len(_greedy(text, size, mid, bold)) == len(lines):
            hi = mid
        else:
            lo = mid
    return _greedy(text, size, hi, bold)


def _greedy(text: str, size: float, max_width: float, bold: bool) -> list[str]:
    lines: list[str] = []
    for para in text.split('\n'):
        tokens = re.findall(r'[A-Za-z0-9_./=+\-*()<>→\[\]{}#:@\'"]+|\s|.', para)
        line = ''
        for tok in tokens:
            # Closing punctuation never starts a line.
            if text_width(line + tok, size, bold) <= max_width or not line.strip() or tok in '、，。；：！？）」』》,.;:!?)':
                line += tok
            else:
                lines.append(line.rstrip())
                line = tok.lstrip()
        lines.append(line.rstrip())
    return lines


def esc(text: str) -> str:
    return html.escape(text, quote=True)


class Diagram:
    def __init__(self, width: int, height: int, title: str, desc: str):
        self.width, self.height = width, height
        self.title, self.desc = title, desc
        self.shapes: list[str] = []
        self.edges: list[str] = []
        self.warnings: list[str] = []

    # Shapes -----------------------------------------------------------------
    def band(self, x, y, w, h, label=None):
        self.shapes.append(f'<rect class="band" x="{x}" y="{y}" width="{w}" height="{h}" rx="10"/>')
        if label:
            self.shapes.append(f'<text class="band-label" x="{x + 14}" y="{y + 19}">{esc(label)}</text>')

    def node(self, x, y, w, h, title, sub=None, kind='node', title_size=14):
        """A box with a bold title and an optional smaller line of detail, centered vertically."""
        rx = h / 2 if kind in ('end', 'stop') and h <= 44 else 8
        self.shapes.append(f'<rect class="{kind}" x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}"/>')
        inner = w - 12
        title_lines = wrap(title, title_size, inner, bold=True) if title else []
        sub_lines = wrap(sub, 12, inner) if sub else []
        heights = [title_size + 4] * len(title_lines) + [16] * len(sub_lines)
        total = sum(heights) + (2 if title_lines and sub_lines else 0)
        if total > h - 8:
            self.warnings.append(f'text does not fit in "{title}" ({total:.0f} > {h - 8})')
        for line in title_lines + sub_lines:
            if text_width(line, 12) > inner + 2:
                self.warnings.append(f'line too wide in "{title}": {line}')
        cy = y + (h - total) / 2
        cx = x + w / 2
        for line in title_lines:
            cy += title_size + 4
            size = '' if title_size == 14 else f' style="font-size:{title_size}px"'
            self.shapes.append(f'<text class="t" x="{cx}" y="{cy - 4:.1f}"{size}>{esc(line)}</text>')
        if title_lines and sub_lines:
            cy += 2
        for line in sub_lines:
            cy += 16
            self.shapes.append(f'<text class="s" x="{cx}" y="{cy - 4:.1f}">{esc(line)}</text>')
        return (x, y, w, h)

    def store(self, x, y, w, h, title, sub=None):
        ry = 10
        self.shapes.append(f'<path class="node" d="M{x} {y + ry}a{w / 2} {ry} 0 0 1 {w} 0V{y + h - ry}a{w / 2} {ry} 0 0 1 {-w} 0z"/>')
        self.shapes.append(f'<path class="edge" style="stroke: var(--node-line)" d="M{x} {y + ry}a{w / 2} {ry} 0 0 0 {w} 0"/>')
        self.node_text(x, y + 2 * ry, w, h - 2 * ry, title, sub)

    def node_text(self, x, y, w, h, title, sub=None):
        lines = [('t', l) for l in wrap(title, 14, w - 16, True)] + [('s', l) for l in (wrap(sub, 12, w - 16) if sub else [])]
        total = sum(18 if k == 't' else 16 for k, _ in lines)
        cy = y + (h - total) / 2
        for k, l in lines:
            cy += 18 if k == 't' else 16
            self.shapes.append(f'<text class="{k}" x="{x + w / 2}" y="{cy - 4:.1f}">{esc(l)}</text>')

    def text(self, x, y, text, cls='el', anchor='start', rotate=None):
        transform = f' transform="rotate({rotate} {x} {y})"' if rotate else ''
        self.edges.append(f'<text class="{cls}" x="{x}" y="{y}" text-anchor="{anchor}"{transform}>{esc(text)}</text>')

    def raw(self, svg: str, on_top=False):
        (self.edges if on_top else self.shapes).append(svg)

    # Edges ------------------------------------------------------------------
    def edge(self, *points, label=None, at=None, anchor='start', back=False, both=False, band=False, radius=6, arrow=True, dashed=False):
        """An orthogonal or straight polyline through the points, with rounded corners."""
        d = path(points, radius)
        cls = 'back' if back else 'edge'
        marker = 'b' if back else 'a'
        start = f' marker-start="url(#{marker})"' if both else ''
        end = f' marker-end="url(#{marker})"' if arrow else ''
        dash = ' stroke-dasharray="5 4"' if dashed else ''
        self.edges.append(f'<path class="{cls}" d="{d}"{dash}{start}{end}/>')
        if label:
            lx, ly = at if at else (points[0][0] + 8, (points[0][1] + points[1][1]) / 2 + 4)
            cls = 'el bl' if back else 'el'
            if band:
                cls += ' el-band'
            for i, line in enumerate(label.split('\n')):
                self.text(lx, ly + i * 14, line, cls, anchor)

    def render(self) -> str:
        if self.warnings:
            raise SystemExit(f'{self.title}:\n  ' + '\n  '.join(self.warnings))
        body = '\n  '.join(self.shapes + self.edges)
        return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.width} {self.height}" '
                f'width="{self.width}" height="{self.height}" role="img" aria-labelledby="title desc">\n'
                f'  <title id="title">{esc(self.title)}</title>\n  <desc id="desc">{esc(self.desc)}</desc>\n'
                f'  <style>{STYLE}</style>\n  {DEFS}\n'
                f'  <rect class="bg" width="{self.width}" height="{self.height}" rx="12"/>\n  {body}\n</svg>\n')


def path(points, radius=6) -> str:
    """Polyline with rounded corners; the last segment keeps its full length for the arrowhead."""
    pts = [tuple(map(float, p)) for p in points]
    out = [f'M{fmt(pts[0][0])} {fmt(pts[0][1])}']
    for i in range(1, len(pts) - 1):
        (x0, y0), (x1, y1), (x2, y2) = pts[i - 1], pts[i], pts[i + 1]
        l1 = ((x1 - x0) ** 2 + (y1 - y0) ** 2) ** 0.5
        l2 = ((x2 - x1) ** 2 + (y2 - y1) ** 2) ** 0.5
        r = min(radius, l1 / 2, l2 / 2)
        ax, ay = x1 - (x1 - x0) / l1 * r, y1 - (y1 - y0) / l1 * r
        bx, by = x1 + (x2 - x1) / l2 * r, y1 + (y2 - y1) / l2 * r
        out.append(f'L{fmt(ax)} {fmt(ay)}Q{fmt(x1)} {fmt(y1)} {fmt(bx)} {fmt(by)}')
    out.append(f'L{fmt(pts[-1][0])} {fmt(pts[-1][1])}')
    return ''.join(out)


def fmt(v: float) -> str:
    return f'{v:.1f}'.rstrip('0').rstrip('.')


# Sequence diagrams ------------------------------------------------------------

class Sequence:
    """Lays out a sequence diagram top to bottom from a list of steps.

    Steps: ('msg', a, b, text) and ('reply', a, b, text) for calls and returns,
    ('self', a, text) for work inside one participant, ('note', a, b, text),
    and ('block', kind, [(label, steps), ...]) for alt / opt / loop frames.
    """

    def __init__(self, title, desc, participants, width=720, head_h=52):
        self.title, self.desc = title, desc
        self.parts = participants  # [(key, title, sub)]
        self.width = width
        n = len(participants)
        margin = 16
        self.col = (width - 2 * margin) / n
        self.x = {key: margin + self.col * (i + 0.5) for i, (key, *_rest) in enumerate(participants)}
        self.items: list[str] = []
        self.y = 0.0
        self.depth = 0
        self.head_h = head_h

    def build(self, steps) -> Diagram:
        head_h = self.head_h
        self.y = 16 + head_h + 18
        self._steps(steps, 16, self.width - 16)
        height = int(self.y + 16)
        d = Diagram(self.width, height, self.title, self.desc)
        for key, title, sub in self.parts:
            x = self.x[key]
            w = self.col - 10
            d.raw(f'<line class="life" x1="{fmt(x)}" y1="{16 + head_h}" x2="{fmt(x)}" y2="{height - 16}"/>')
            d.node(x - w / 2, 16, w, head_h, title, sub)
        for item in self.items:
            d.raw(item, on_top=True)
        return d

    def _label_lines(self, text, max_w):
        return wrap(text, 12, max_w)

    def _steps(self, steps, left, right):
        for step in steps:
            kind = step[0]
            if kind in ('msg', 'reply'):
                _, a, b, text = step
                xa, xb = self.x[a], self.x[b]
                span = abs(xb - xa)
                lines = self._label_lines(text, max(span + self.col * 0.5, 150))
                half = max(text_width(l, 12) for l in lines) / 2
                lx = min(max((xa + xb) / 2, 16 + half), self.width - 16 - half)
                for i, line in enumerate(lines):
                    self.items.append(f'<text class="el" x="{fmt(lx)}" y="{fmt(self.y + 12 + i * 15)}" text-anchor="middle" style="font-size:12px">{esc(line)}</text>')
                self.y += 15 * len(lines) + 6
                cls = 'back' if kind == 'reply' else 'edge'
                marker = 'b' if kind == 'reply' else 'a'
                end = xb - 2 if xb > xa else xb + 2
                self.items.append(f'<path class="{cls}" d="M{fmt(xa)} {fmt(self.y)}H{fmt(end)}" marker-end="url(#{marker})"/>')
                self.y += 14
            elif kind == 'self':
                _, a, text = step
                x = self.x[a]
                max_w = min(self.col * 1.9, 260)
                lines = wrap(text, 12, max_w - 16)
                w = max(text_width(l, 12) for l in lines) + 20
                h = 15 * len(lines) + 10
                bx = min(max(x - w / 2, left + 4), right - 4 - w)
                self.items.append(f'<rect class="node" x="{fmt(bx)}" y="{fmt(self.y)}" width="{fmt(w)}" height="{fmt(h)}" rx="6"/>')
                for i, line in enumerate(lines):
                    self.items.append(f'<text class="s" x="{fmt(bx + w / 2)}" y="{fmt(self.y + 18 + i * 15)}" style="fill: var(--title)">{esc(line)}</text>')
                self.y += h + 12
            elif kind == 'note':
                _, a, b, text = step
                x0, x1 = min(self.x[a], self.x[b]) - self.col * 0.42, max(self.x[a], self.x[b]) + self.col * 0.42
                x0, x1 = max(x0, left + 4), min(x1, right - 4)
                lines = wrap(text, 12, x1 - x0 - 16)
                h = 15 * len(lines) + 12
                self.items.append(f'<rect class="note" x="{fmt(x0)}" y="{fmt(self.y)}" width="{fmt(x1 - x0)}" height="{fmt(h)}" rx="6"/>')
                for i, line in enumerate(lines):
                    self.items.append(f'<text class="s" x="{fmt((x0 + x1) / 2)}" y="{fmt(self.y + 19 + i * 15)}" style="fill: var(--title)">{esc(line)}</text>')
                self.y += h + 12
            elif kind == 'block':
                _, tag, branches = step
                inset = 8 * self.depth
                bx0, bx1 = left + inset, right - inset
                top = self.y
                self.depth += 1
                splits = []
                for i, (label, inner) in enumerate(branches):
                    if i:
                        splits.append(self.y)
                    tag_text = tag if i == 0 else 'else'
                    tw = text_width(tag_text, 11) + 14
                    self.items.append(f'<path class="block-tag" d="M{fmt(bx0)} {fmt(self.y)}h{fmt(tw)}v14l-5 5h{fmt(-tw + 5)}z"/>')
                    self.items.append(f'<text class="block-tag-text" x="{fmt(bx0 + 7)}" y="{fmt(self.y + 13)}">{esc(tag_text)}</text>')
                    self.items.append(f'<text class="block-label" x="{fmt(bx0 + tw + 8)}" y="{fmt(self.y + 14)}">{esc(label)}</text>')
                    self.y += 30
                    self._steps(inner, bx0 + 4, bx1 - 4)
                self.depth -= 1
                self.y += 2
                self.items.insert(0, f'<rect class="block" x="{fmt(bx0)}" y="{fmt(top)}" width="{fmt(bx1 - bx0)}" height="{fmt(self.y - top)}" rx="4"/>')
                for sy in splits:
                    self.items.insert(1, f'<line class="block-split" x1="{fmt(bx0)}" y1="{fmt(sy - 4)}" x2="{fmt(bx1)}" y2="{fmt(sy - 4)}"/>')
                self.y += 12
            else:
                raise ValueError(kind)
