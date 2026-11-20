#!/usr/bin/env python3
"""Writes cases.json, the A/B cases of lm-plugins-guitarix, from what every unit registers:
    cargo run -p lm-sys-guitarix --example dump > units.txt && python3 make-cases.py units.txt > cases.json

Every unit once with every setting moved off its default (a third of the way to its farther end, a switch flipped, a
selector one value on), then two of its linear sliders back to their defaults in the middle (as parameter events);
and every module of the units that are a choice among modules (amps, tonestacks, cabinets, amp impulses, wahs,
power amps, the crybaby), each at its other defaults. 48 kHz; the period is the unit's block: 64 for the convolving
units and the amp head, 256 for the others (the legacy plugin processes in blocks of its period)."""

import json
import re
import sys

FIXED_64 = {"cab", "pre", "ampstack"}
# Every module of these selectors gets a case of its own
SWEEPS = {"amp.tonestack": "amp.tonestack.select", "ampstack": "tube.select", "cab": "cab.select",
          "pre": "pre.select", "wah": "wah.select", "poweramp": "poweramp.mode", "crybaby": "crybaby.autowah"}


def parse(path):
    units, unit = [], None
    for line in open(path, encoding="utf-8"):
        if not line.startswith(" "):
            fields = [f.strip() for f in line.split("|")]
            unit = {"id": fields[0], "stereo": fields[3] == "stereo true", "params": []}
            units.append(unit)
            continue
        f = [x.strip() for x in line.split("|")]
        m = re.match(r"(\S+) \[(\S+) (\S+)\] (\S+)", f[4])
        values = json.loads(f[5])
        unit["params"].append({"id": f[0], "kind": f[2], "var": f[3], "default": float(m.group(1)),
                               "low": float(m.group(2)), "up": float(m.group(3)), "step": float(m.group(4)),
                               "values": values})
    return units


def is_output(p):
    return any(c in p["kind"][1:] for c in "Oo")


def moved(p):
    lo, up, d = p["low"], p["up"], p["default"]
    if p["values"]:
        n = len(p["values"])
        return lo + (round(d - lo) + 1) % n
    if p["var"] in ("Bool", "Int") or (p["step"] >= 1 and float(p["step"]).is_integer()):
        if p["var"] == "Bool" or up - lo <= 1:
            return up if d != up else lo
        v = d + max(1.0, round((up - d) / 3)) if up - d > d - lo else d - max(1.0, round((d - lo) / 3))
        return min(max(v, lo), up)
    v = d + (up - d) / 3 if up - d > d - lo else d - (d - lo) / 3
    return float(f"{v:.4g}")


def main():
    units = parse(sys.argv[1])
    cases = []
    for u in units:
        uid = u["id"]
        block = 64 if uid in FIXED_64 else 256
        settable = [p for p in u["params"] if not is_output(p)]
        settings = {p["id"]: moved(p) for p in settable}
        linear = [p for p in settable if not p["values"] and p["var"] == "Float" and "L" not in p["kind"]
                  and not (p["step"] >= 1 and float(p["step"]).is_integer())
                  and p["id"] not in ("con.Level",)]
        signal = {"kind": "sine", "hz": 196, "amp": 0.25, "right": {"kind": "sine", "hz": 247, "amp": 0.2}}
        if "verb" in uid or "rev" in uid:
            signal = {"kind": "noise", "seed": 7, "amp": 0.2, "right": {"kind": "noise", "seed": 11, "amp": 0.2}}
        case = {"name": "gx-" + re.sub(r"[^a-z0-9]+", "-", uid.lower()).strip("-"), "unit": uid, "rate": 48000,
                "block": block, "frames": 4096, "signal": signal, "settings": settings}
        if linear:
            case["then"] = {"at": block * 8, "settings": {p["id"]: p["default"] for p in linear[:2]}}
        cases.append(case)
        if uid in SWEEPS:
            sel = next(p for p in settable if p["id"] == SWEEPS[uid])
            for i, key in enumerate(sel["values"]):
                cases.append({"name": f"{case['name']}-{re.sub(r'[^a-z0-9]+', '-', key.lower()).strip('-')}-{i}",
                              "unit": uid, "rate": 48000, "block": block, "frames": 2048,
                              "signal": {"kind": "sine", "hz": 165, "amp": 0.25,
                                         "right": {"kind": "sine", "hz": 220, "amp": 0.2}},
                              "settings": {sel["id"]: sel["low"] + i}})
    json.dump({"cases": cases}, sys.stdout, indent=1)
    sys.stdout.write("\n")


main()
