#!/usr/bin/env python3
"""Generates crates/lm-gui-plugins/src/panels/guitarix_units.rs: every Guitarix unit's panel as a table (its title,
size and controls at their places), from the per-unit layout trees in docs/plugins-visual.md (themselves read off
legado/plugins/Guitarix/GuitarixLayout.cpp's GladeReader and the glade files) and from what the plugin registers
(`cargo run -p lm-sys-guitarix --example dump`: the selectors' values and the meters' places).

    python3 tools/gen_guitarix_panels.py docs/plugins-visual.md dump.txt > crates/lm-gui-plugins/src/panels/guitarix_units.rs

The layout is GTK's box rules as docs/plugins-visual.md lists them, worked out at the natural size of each widget
(the rack unit has no other size): boxes pack their children in order with the spacing and the padding, a grid's
columns and rows take the widest and tallest of their cells, a control is centered in its cell.
"""
import re
import sys

KNOB = {  # class: (widget w, widget h)
    "GxSmallKnob": ("Small", 54, 46),
    "GxSmallKnobR": ("SmallR", 84, 76),
    "GxMidKnob": ("Mid", 106, 98),
    "GxBigKnob": ("Big", 130, 142),
}
SWITCH = {"switch": 24, "minitoggle": 18, "switch_mid": 18, "frbutton": 29}
LABEL_H = 14


class Node:
    def __init__(self, typ, line, depth):
        self.typ = typ
        self.line = line
        self.depth = depth
        self.kids = []
        self.tab = None
        self.placed = None


def num(s):
    s = s.strip().rstrip("f")
    return float(s)


def parse_tree(lines):
    roots = []
    stack = []
    for ln in lines:
        m = re.match(r"^(\s*)- (.*)$", ln)
        if not m:
            continue
        depth = len(m.group(1)) // 2
        body = m.group(2)
        if body.startswith("(GtkImage") or body.startswith("(Para") or body.startswith("(sem"):
            continue
        mt = re.match(r'\(aba: "(.*)"\)', body)
        if mt:
            # the tab of the previous sibling
            for n in reversed(stack):
                if n.depth == depth:
                    n.tab = mt.group(1)
                    break
            continue
        typ = body.split()[0]
        node = Node(typ, body, depth)
        while stack and stack[-1].depth >= depth:
            stack.pop()
        if stack:
            stack[-1].kids.append(node)
        else:
            roots.append(node)
        stack.append(node)
    return roots


def prop(node, name, default=None):
    m = re.search(r"\b" + name + r"=([^\s{}<>]+)", node.line)
    return m.group(1) if m else default


def props(node):
    m = re.search(r"\{([^}]*)\}", node.line)
    d = {}
    if m:
        for kv in m.group(1).split():
            if "=" in kv:
                k, v = kv.split("=", 1)
                d[k] = v
    return d


def packing(node):
    m = re.search(r"<([^>]*)>", node.line)
    d = {}
    if m:
        for kv in m.group(1).split():
            if "=" in kv:
                k, v = kv.split("=", 1)
                d[k] = v
    return d


def truthy(v):
    return str(v).lower() in ("true", "t", "1")


def ident(node):
    m = re.search(r"`([^`]*)`", node.line)
    return m.group(1) if m else ""


def quoted(node):
    m = re.search(r'"((?:[^"\\]|\\.)*)"', node.line)
    return m.group(1) if m else ""


def is_ctrl(node):
    return node.typ.startswith("Gx") and node.typ != "GxPaintBox"


def ctrl_info(node):
    """var id, name, type text of a control"""
    vid = ident(node)
    name = quoted(node)
    mt = re.search(r"\[([^\]]*)\]", node.line)
    t = mt.group(1) if mt else ""
    return vid, name, t


def parse_range(t):
    """(is_float, options, def, min, max, step) from 'Float SL; 0.0,0.0,1.0,0.01'"""
    head, _, rest = t.partition(";")
    words = head.split()
    typ = words[0] if words else "Float"
    opts = words[1] if len(words) > 1 else ""
    nums = re.findall(r"-?\d+(?:\.\d+)?(?:e[+-]?\d+)?f?", rest.split("(")[0])
    vals = []
    for n in nums[:4]:
        try:
            vals.append(num(n))
        except ValueError:
            pass
    if typ == "Bool" or (len(vals) < 4):
        if typ == "Bool":
            return typ, opts, 0.0, 0.0, 1.0, 1.0
    if len(vals) >= 4:
        return typ, opts, vals[0], vals[1], vals[2], vals[3]
    return typ, opts, 0.0, 0.0, 0.0, 0.0


def text_w(s, px):
    return len(s) * px * 0.46 + 2


class Ctx:
    def __init__(self, unit, dump):
        self.unit = unit
        self.dump = dump  # param key -> info
        self.refs = {}  # label id -> name
        self.items = []
        self.tabs = []  # (name, x, y, w, h)
        self.in_notebook = False
        self.view = 255


def collect_refs(node, ctx):
    if is_ctrl(node):
        m = re.search(r"label_ref=`([^`]*)`", node.line)
        if m:
            ctx.refs[m.group(1)] = quoted(node)
    for k in node.kids:
        collect_refs(k, ctx)


def label_text(node, ctx, nb):
    ident_ = ident(node)
    cls = ""
    if ":" in ident_:
        cls = ident_.split(":", 1)[1]
    if ident_ in ctx.refs:
        txt = ctx.refs[ident_]
    else:
        txt = quoted(node)
    if "inverse" in cls or nb:
        px, color = 9, 0xAAAAAA
    elif "beffekt" in cls:
        px, color = 11, 0x222222
    else:
        px, color = 11, 0x222222
    return txt, px, color


def leaf_size(node, ctx, nb):
    t = node.typ
    if t == "Label":
        txt, px, _ = label_text(node, ctx, nb)
        if not txt.strip():
            return (0, 0)
        return (text_w(txt, px), px + 3)
    if t in KNOB:
        return (KNOB[t][1], KNOB[t][2])
    if t == "GxSwitch":
        base = prop(node, "base_name", "switch")
        s = SWITCH.get(base, 24)
        return (s, s)
    if t == "GxSelector":
        vid, name, ty = ctrl_info(node)
        info = ctx.dump.get(key_of(vid))
        widest = 40
        if info and info["values"]:
            widest = max(text_w(v[1], 10) for v in info["values"])
        elif "enum" in ty:
            widest = 90
        return (widest + 36, 24)
    if t == "GxHSlider":
        return (134, 24)
    if t == "GxVSlider":
        return (24, 134)
    if t == "GxFastMeter":
        h = prop(node, "orient", "vertical") == "horizontal"
        return (60, 7) if h else (7, 60)
    if t == "GxValueDisplay":
        d = int(prop(node, "round_digits", "0") or 0)
        return (text_w("-000." + "0" * d, 11) + 14, 20)
    if t == "GxPortDisplay":
        return (40, 7)
    return (0, 0)


def key_of(vid):
    k = "".join(c if (c.isascii() and c.isalnum()) or c == "_" else "_" for c in vid)
    if not k[:1].isalpha():
        k = "p" + k
    return k


def margins(node):
    p = props(node)
    return (
        int(p.get("margin_left", 0)),
        int(p.get("margin_top", 0)),
        int(p.get("margin_right", 0)),
        int(p.get("margin_bottom", 0)),
    )


def requested(node):
    p = props(node)
    return int(p.get("width_request", 0)), int(p.get("height_request", 0))


def size(node, ctx, nb=False):
    ml, mt, mr, mb = margins(node)
    rw, rh = requested(node)
    w, h = size_inner(node, ctx, nb)
    w, h = max(w, rw), max(h, rh)
    return (w + ml + mr, h + mt + mb)


def kids_of(node):
    return [k for k in node.kids if k.typ != "Para"]


def size_inner(node, ctx, nb):
    t = node.typ
    if t in ("GtkBox", "GxPaintBox"):
        vertical = "V" in node.line.split()[1:2] or " V " in node.line[:30]
        sp = int(prop(node, "sp", "0") or 0)
        kids = kids_of(node)
        ws, hs = [], []
        for k in kids:
            w, h = size(k, ctx, nb)
            pad = int(packing(k).get("padding", 0)) * 2
            if vertical:
                ws.append(w)
                hs.append(h + pad)
            else:
                ws.append(w + pad)
                hs.append(h)
        n = len(kids)
        if n == 0:
            return (0, 0)
        if vertical:
            return (max(ws), sum(hs) + sp * (n - 1))
        return (sum(ws) + sp * (n - 1), max(hs))
    if t == "GtkGrid":
        cols, rows = grid_dims(node, ctx, nb)
        csp = int(prop(node, "colsp", "0") or 0)
        rsp = int(prop(node, "rowsp", "0") or 0)
        return (sum(cols) + csp * max(len(cols) - 1, 0), sum(rows) + rsp * max(len(rows) - 1, 0))
    if t == "GtkAlignment":
        m = re.search(r"pad L(\d+) T(\d+) R(\d+) B(\d+)", node.line)
        l, tp, r, b = (int(x) for x in m.groups()) if m else (0, 0, 0, 0)
        kids = kids_of(node)
        w, h = size(kids[0], ctx, nb) if kids else (0, 0)
        return (w + l + r, h + tp + b)
    if t == "GtkFrame":
        kids = kids_of(node)
        w, h = size(kids[0], ctx, nb) if kids else (0, 0)
        lab = quoted(node) if 'label="' in node.line else ""
        lh = 12 if lab else 6
        return (w + 8, h + lh + 4)
    if t == "GtkNotebook":
        pages = kids_of(node)
        sz = [size(p, ctx, True) for p in pages]
        w = max([s[0] for s in sz] or [0]) + 18
        h = max([s[1] for s in sz] or [0]) + 24 + 14
        tabs_w = sum(text_w(p.tab or "", 9) + 16 for p in pages)
        return (max(w, tabs_w + 8), h)
    return leaf_size(node, ctx, nb)


def grid_dims(node, ctx, nb):
    homog = prop(node, "homog", "F/F").split("/")
    hc = truthy(homog[0]) or homog[0] == "T"
    hr = truthy(homog[1]) if len(homog) > 1 else False
    cols, rows = {}, {}
    for k in kids_of(node):
        p = packing(k)
        c, r = int(p.get("left_attach", 0)), int(p.get("top_attach", 0))
        cw, rh_ = int(p.get("width", 1)), int(p.get("height", 1))
        w, h = size(k, ctx, nb)
        if cw == 1:
            cols[c] = max(cols.get(c, 0), w)
        if rh_ == 1:
            rows[r] = max(rows.get(r, 0), h)
    nc = max(cols) + 1 if cols else 1
    nr = max(rows) + 1 if rows else 1
    # spanning children widen their last column when the columns they span are narrower
    csp = int(prop(node, "colsp", "0") or 0)
    rsp = int(prop(node, "rowsp", "0") or 0)
    for k in kids_of(node):
        p = packing(k)
        c, r = int(p.get("left_attach", 0)), int(p.get("top_attach", 0))
        cw, rh_ = int(p.get("width", 1)), int(p.get("height", 1))
        w, h = size(k, ctx, nb)
        nc = max(nc, c + cw)
        nr = max(nr, r + rh_)
        if cw > 1:
            have = sum(cols.get(i, 0) for i in range(c, c + cw)) + csp * (cw - 1)
            if w > have:
                cols[c + cw - 1] = cols.get(c + cw - 1, 0) + (w - have)
        if rh_ > 1:
            have = sum(rows.get(i, 0) for i in range(r, r + rh_)) + rsp * (rh_ - 1)
            if h > have:
                rows[r + rh_ - 1] = rows.get(r + rh_ - 1, 0) + (h - have)
    cl = [cols.get(i, 0) for i in range(nc)]
    rl = [rows.get(i, 0) for i in range(nr)]
    if hc:
        cl = [max(cl)] * nc
    if hr:
        rl = [max(rl)] * nr
    return cl, rl


def emit(ctx, kind, node, x, y, w, h, nb, extra=None):
    ctx.items.append(dict(kind=kind, node=node, x=x, y=y, w=w, h=h, view=ctx.view, nb=nb, extra=extra))


def place(node, ctx, x, y, w, h, nb=False):
    """Places `node` in the area (x, y, w, h): the area is its own when it fills; a leaf is centered in it"""
    ml, mt, mr, mb = margins(node)
    x, y, w, h = x + ml, y + mt, w - ml - mr, h - mt - mb
    t = node.typ
    p = props(node)
    if t == "GxPaintBox" and "box_uni_2" in node.line:
        emit(ctx, "dark", node, x, y, w, h, nb)
    nw, nh = size_inner(node, ctx, nb)
    rw, rh = requested(node)
    nw, nh = max(nw, rw), max(nh, rh)
    if t in ("GtkBox", "GxPaintBox"):
        vertical = " V " in node.line[:30]
        sp = int(prop(node, "sp", "0") or 0)
        kids = kids_of(node)
        infos = []
        for k in kids:
            pk = packing(k)
            kp = props(k)
            kw, kh = size(k, ctx, nb)
            pad = int(pk.get("padding", 0))
            expand = truthy(pk.get("expand", "False")) or truthy(kp.get("vexpand" if vertical else "hexpand", "False"))
            fill = truthy(pk.get("fill", "True"))
            end = pk.get("pack_type") == "end"
            infos.append(dict(k=k, kw=kw, kh=kh, pad=pad, expand=expand, fill=fill, end=end, kp=kp))
        main_total = sum((i["kh"] if vertical else i["kw"]) + 2 * i["pad"] for i in infos) + sp * max(len(infos) - 1, 0)
        avail = h if vertical else w
        extra = max(avail - main_total, 0)
        nexp = sum(1 for i in infos if i["expand"])
        order = [i for i in infos if not i["end"]] + [i for i in reversed([i for i in infos if i["end"]])]
        # a gap between the start children and the end children when nothing expands
        pos = y if vertical else x
        gap_at = len([i for i in infos if not i["end"]])
        for idx, i in enumerate(order):
            if idx == gap_at and nexp == 0 and any(j["end"] for j in infos):
                pos += extra
            share = extra / nexp if nexp and i["expand"] else 0
            kmain = (i["kh"] if vertical else i["kw"]) + (share if i["fill"] else 0)
            cell_main = (i["kh"] if vertical else i["kw"]) + share
            pos += i["pad"]
            kcross_avail = w if vertical else h
            kcross = i["kw"] if vertical else i["kh"]
            if i["fill"] and not is_leaf(i["k"]):
                kcross = kcross_avail
            al = i["kp"].get("halign" if vertical else "valign", "")
            off = 0
            if al == "start":
                off = 0
            elif al == "end":
                off = kcross_avail - kcross
            else:
                off = (kcross_avail - kcross) / 2
            if vertical:
                place(i["k"], ctx, x + off, pos, kcross, cell_main if i["fill"] else i["kh"], nb)
            else:
                place(i["k"], ctx, pos, y + off, cell_main if i["fill"] else i["kw"], kcross, nb)
            pos += cell_main + i["pad"] + sp
        return
    if t == "GtkGrid":
        cols, rows = grid_dims(node, ctx, nb)
        csp = int(prop(node, "colsp", "0") or 0)
        rsp = int(prop(node, "rowsp", "0") or 0)
        gw = sum(cols) + csp * max(len(cols) - 1, 0)
        gh = sum(rows) + rsp * max(len(rows) - 1, 0)
        # expanding columns and rows share the extra room
        ex_c = [False] * len(cols)
        ex_r = [False] * len(rows)
        for k in kids_of(node):
            pk = packing(k)
            kp = props(k)
            c, r = int(pk.get("left_attach", 0)), int(pk.get("top_attach", 0))
            if truthy(kp.get("hexpand", "False")) and c < len(cols):
                ex_c[c] = True
            if truthy(kp.get("vexpand", "False")) and r < len(rows):
                ex_r[r] = True
        if w > gw and any(ex_c):
            add = (w - gw) / sum(ex_c)
            cols = [cw + (add if e else 0) for cw, e in zip(cols, ex_c)]
            gw = w
        if h > gh and any(ex_r):
            add = (h - gh) / sum(ex_r)
            rows = [rh_ + (add if e else 0) for rh_, e in zip(rows, ex_r)]
            gh = h
        ox = x + max((w - gw) / 2, 0) if not any(ex_c) and truthy(p.get("halign") == "center") else x
        for k in kids_of(node):
            pk = packing(k)
            kp = props(k)
            c, r = int(pk.get("left_attach", 0)), int(pk.get("top_attach", 0))
            cw_, rh_ = int(pk.get("width", 1)), int(pk.get("height", 1))
            cx_ = ox + sum(cols[:c]) + csp * c
            cy_ = y + sum(rows[:r]) + rsp * r
            cwid = sum(cols[c : c + cw_]) + csp * (cw_ - 1)
            chei = sum(rows[r : r + rh_]) + rsp * (rh_ - 1)
            kw, kh = size(k, ctx, nb)
            if not is_leaf(k):
                al = kp.get("halign", "")
                if al not in ("start", "end", "center"):
                    place(k, ctx, cx_, cy_, cwid, chei, nb)
                    continue
            ha = kp.get("halign", "center")
            if ha == "start":
                kx = cx_
            elif ha == "end":
                kx = cx_ + cwid - kw
            else:
                kx = cx_ + (cwid - kw) / 2
            va = kp.get("valign", "center")
            if va == "start":
                ky = cy_
            elif va == "end":
                ky = cy_ + chei - kh
            else:
                ky = cy_ + (chei - kh) / 2
            place(k, ctx, kx, ky, kw, kh, nb)
        return
    if t == "GtkAlignment":
        m = re.search(r"pad L(\d+) T(\d+) R(\d+) B(\d+)", node.line)
        l, tp, r, b = (int(v) for v in m.groups()) if m else (0, 0, 0, 0)
        kids = kids_of(node)
        if kids:
            place(kids[0], ctx, x + l, y + tp, w - l - r, h - tp - b, nb)
        return
    if t == "GtkFrame":
        kids = kids_of(node)
        lab = quoted(node) if 'label="' in node.line else ""
        lh = 12 if lab else 6
        if lab:
            emit(ctx, "label", node, x + 4, y, text_w(lab, 9), 12, nb, (lab, 9, 0xAAAAAA))
        if kids:
            place(kids[0], ctx, x + 4, y + lh, w - 8, h - lh - 4, nb)
        return
    if t == "GtkNotebook":
        pages = kids_of(node)
        # the tabs
        tx = x + 4
        for n, pg in enumerate(pages):
            tw = text_w(pg.tab or "", 9) + 16
            ctx.tabs.append((pg.tab or "", n, tx, y, tw, 20))
            tx += tw
        old = ctx.view
        for n, pg in enumerate(pages):
            ctx.view = n
            place(pg, ctx, x + 9, y + 26, w - 18, h - 24 - 14, True)
        ctx.view = old
        return
    # a leaf
    kw, kh = size_inner(node, ctx, nb)
    if is_leaf(node):
        px_, py_ = x + (w - kw) / 2, y + (h - kh) / 2
        if node.typ == "GxFastMeter":
            # a meter fills the room it is given along its length
            horizontal = prop(node, "orient", "vertical") == "horizontal"
            if horizontal:
                kw = max(kw, min(w, 120))
                px_ = x + (w - kw) / 2
            else:
                kh = max(kh, min(h, 120))
                py_ = y + (h - kh) / 2
        emit(ctx, "leaf", node, px_, py_, kw, kh, nb)
        return
    # anything else with children: the first child fills
    kids = kids_of(node)
    if kids:
        place(kids[0], ctx, x, y, w, h, nb)


def is_leaf(node):
    return node.typ == "Label" or is_ctrl(node)


def rs(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def f(v):
    return "%.1f" % v


def read_dump(path):
    units = {}
    cur = None
    for ln in open(path):
        if ln.startswith("    "):
            parts = [x.strip() for x in ln.strip().split(" | ")]
            if len(parts) < 6 or cur is None:
                continue
            pid, name, kind = parts[0], parts[1], parts[2]
            vals = []
            tail = ln.rsplit(" | ", 1)[-1]
            vals = re.findall(r'"((?:[^"\\]|\\.)*)"', tail)
            keys = []
            for n_, v in enumerate(vals):
                k = v.strip()
                if not k or len(k) > 64 or k in keys:
                    k = str(n_)
                keys.append(k)
            cur["params"][key_of(pid)] = dict(id=pid, name=name, kind=kind, values=list(zip(keys, vals)))
            cur["order"].append(key_of(pid))
        elif "|" in ln:
            parts = [x.strip() for x in ln.split(" | ")]
            cur = dict(id=parts[0], name=parts[1], params={}, order=[])
            units[parts[0]] = cur
    return units


def main():
    doc = open(sys.argv[1]).read().split("\n")
    dump = read_dump(sys.argv[2]) if len(sys.argv) > 2 else {}
    start = next(i for i, l in enumerate(doc) if l.startswith("### Guitarix"))
    end = next(i for i, l in enumerate(doc) if l.startswith("### ") and i > start)
    sections = []
    cur = None
    for l in doc[start:end]:
        m = re.match(r'#### \d+\. `([^`]+)` — "([^"]*)"', l)
        if m:
            cur = dict(unit=m.group(1), title=m.group(2), lines=[])
            sections.append(cur)
        elif cur is not None:
            cur["lines"].append(l)
    out = []
    out.append("//! GENERATED by tools/gen_guitarix_panels.py from docs/plugins-visual.md and the unit registry: do not edit.")
    out.append("//! Every Guitarix unit's panel as a table: its title, its size and each control at its place.")
    out.append("")
    out.append("use super::guitarix::{Item, K, Unit, i};")
    out.append("")
    unit_names = []
    for sec in sections:
        u = sec["unit"]
        d = dump.get(u, dict(params={}, order=[], id=u, name=u))
        ctx = Ctx(u, d["params"])
        roots = parse_tree(sec["lines"])
        if not roots:
            roots = []
        for r in roots:
            collect_refs(r, ctx)
        # the roots stack vertically (the amp head's two parts)
        heights = [size(r, ctx) for r in roots]
        cw = max([h[0] for h in heights] or [0])
        ch = sum(h[1] for h in heights) + 4 * max(len(roots) - 1, 0)
        panel_w = max(cw + 64, 200)
        panel_h = 31 + 4 + ch + 4
        ox = 32 + (panel_w - 64 - cw) / 2
        yy = 35.0
        for r, (w, h) in zip(roots, heights):
            place(r, ctx, ox, yy, cw, h)
            yy += h + 4
        outs = [k for k in d["order"] if is_output(d["params"][k])]
        lines = []
        for it in ctx.items:
            n = it["node"]
            x, y, w, h = it["x"], it["y"], it["w"], it["h"]
            v = it["view"]
            if it["kind"] == "dark":
                lines.append(row("Dark", "", "", x, y, w, h, v, 0, 0.0, 0, 0, ""))
                continue
            if it["kind"] == "label":
                txt, px, color = it["extra"]
                lines.append(row("Label", "", txt, x, y, w, h, v, 0, 0.0, px, color, ""))
                continue
            if n.typ == "Label":
                txt, px, color = label_text(n, ctx, it["nb"])
                if not txt.strip():
                    continue
                lines.append(row("Label", "", txt, x, y, w, h, v, 0, 0.0, px, color, ""))
                continue
            vid, name, ty = ctrl_info(n)
            key = key_of(vid)
            info = d["params"].get(key)
            typ_, opts, dflt, lo, hi, step = parse_range(ty)
            if n.typ == "GxFastMeter":
                if key not in outs:
                    continue
                horizontal = prop(n, "orient", "vertical") == "horizontal"
                lg = ("L" in opts) or step != 0
                lines.append(row("Meter", key, name, x, y, w, h, v, 0, 0.0, 0, pack_meter(outs.index(key), horizontal, lg, lo, hi), ""))
                continue
            if info is None:
                continue
            cond = amp_cond(u, key, n.typ)
            if n.typ in KNOB:
                if info["values"]:
                    continue
                lines.append(row(KNOB[n.typ][0], key, name, x, y, w, h, v, cond, step, int("L" in opts), 0, ""))
            elif n.typ == "GxSwitch":
                base = prop(n, "base_name", "switch")
                kind = {"switch": "Switch", "minitoggle": "MiniToggle", "switch_mid": "SwitchMid", "frbutton": "FrButton"}.get(base, "Switch")
                lines.append(row(kind, key, name, x, y, w, h, v, cond, 1.0, 0, 0, ""))
            elif n.typ == "GxSelector":
                if info["values"]:
                    opts_ = ", ".join("(%s, %s)" % (rs(a), rs(b)) for a, b in info["values"])
                    lines.append(row("Selector", key, name, x, y, w, h, v, cond, 0.0, 0, 0, opts_))
                elif hi - lo == 1 and step == 1:
                    lines.append(row("Switch", key, name, x, y, 24, 24, v, cond, 1.0, 0, 0, ""))
                else:
                    lines.append(row("SmallR", key, name, x, y, 84, 76, v, cond, step or 1.0, 0, 0, ""))
            elif n.typ == "GxVSlider":
                lines.append(row("VSlider", key, name, x, y, 24, 134, v, cond, step, int("L" in opts), 0, ""))
            elif n.typ == "GxHSlider":
                lines.append(row("HSlider", key, name, x, y, 134, 24, v, cond, step, int("L" in opts), 0, ""))
            elif n.typ == "GxValueDisplay":
                dg = int(prop(n, "round_digits", "0") or 0)
                lines.append(row("Value", key, name, x, y, w, h, v, cond, 0.0, dg, 0, ""))
        for (tab, n, tx, ty, tw, th) in ctx.tabs:
            lines.append(row("Tab", "", tab, tx, ty, tw, th, n, 0, 0.0, 0, 0, ""))
        name = "U_" + re.sub(r"[^A-Za-z0-9]", "_", u).upper()
        out.append("const %s: &[Item] = &[" % name)
        out.extend(lines)
        out.append("];")
        out.append("")
        ctx_units.append((u, sec["title"], panel_w, panel_h, name))
    out.append("/// Every unit: Guitarix's id, its title, its size and its controls")
    out.append("pub static UNITS: &[Unit] = &[")
    for (u, title, w, h, name) in ctx_units:
        out.append("    Unit { unit: %s, title: %s, w: %s, h: %s, items: %s }," % (rs(u), rs(title), f(w), f(h), name))
    out.append("];")
    print("\n".join(out))


ctx_units = []


def is_output(p):
    return "O" in p["kind"] or "o" in p["kind"]


def pack_meter(idx, horizontal, lg, lo, hi):
    lo = max(-2048, min(2047, int(lo))) & 0xFFF
    hi = max(-2048, min(2047, int(hi))) & 0xFFF
    return idx | (int(horizontal) << 4) | (int(lg) << 5) | (lo << 6) | (hi << 18)


def amp_cond(unit, key, typ):
    """`showAmpControls`: with the last amp ("---", gxnoamp) only the big gain knob shows; with an amp the medium
    knobs show instead (1: only without an amp, 2: only with one)"""
    if unit != "ampstack":
        return 0
    if typ == "GxBigKnob" and key == "amp2_stage2_gain1":
        return 1
    if typ == "GxSmallKnobR" and key in ("amp2_stage1_Pregain", "gxdistortion_drive", "gxdistortion_wet_dry", "amp2_stage2_gain1"):
        return 2
    return 0


def row(kind, key, name, x, y, w, h, view, cond, step, a, b, opts):
    return "    i(K::%s, %s, %s, %s, %s, %s, %s, %d, %d, %s, %d, %d, &[%s])," % (
        kind, rs(key), rs(name), f(x), f(y), f(w), f(h), view, cond, repr(float(step)), a, b, opts)


if __name__ == "__main__":
    main()
