#!/usr/bin/env python3
"""Makes the Drum Machines instrument's emulated kits (data/samples/drumsynth/dmx, tr707): DrumSynth patches of their
own, each from the program's DrumSynth patch nearest to the machine's voice (data/samples/drumsynth), retuned, and
given the machine's sampling: the Oberheim DMX's 8 bits, the Roland TR-707's 6 bits, both near 22 kHz (DrumSynth's
own bit and rate reduction). They are emulations: the machines' own sounds are their makers' recordings, not free.

usage: make-ds-kits.py (from anywhere: the paths are the repository's)
"""
import os, re

root = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
ds = os.path.join(root, "data", "samples", "drumsynth")

def read(path):
    sections, order, current = {}, [], None
    for line in open(path, encoding="latin-1").read().splitlines():
        line = line.strip()
        m = re.match(r"\[(.+)\]", line)
        if m:
            current = m.group(1)
            sections[current] = []
            order.append(current)
        elif "=" in line and current:
            key, value = line.split("=", 1)
            sections[current].append([key, value])
    return order, sections

def get(sections, section, key, default=None):
    for k, v in sections.get(section, []):
        if k == key:
            return v
    return default

def put(sections, section, key, value):
    for item in sections.setdefault(section, []):
        if item[0] == key:
            item[1] = str(value)
            return
    sections[section].append([key, str(value)])

def write(path, order, sections):
    lines = []
    for section in order:
        lines.append("[%s]" % section)
        lines += ["%s=%s" % (k, v) for k, v in sections[section]]
        lines.append("")
    open(path, "w", encoding="latin-1").write("\n".join(lines))

def make(kit, bits, rate, pieces):
    out = os.path.join(ds, kit)
    os.makedirs(out, exist_ok=True)
    for name, source, tuning, stretch in pieces:
        order, sections = read(os.path.join(ds, source + ".ds"))
        put(sections, "General", "Tuning", "%.2f" % (float(get(sections, "General", "Tuning", "0")) + tuning))
        put(sections, "General", "Stretch", "%.1f" % (float(get(sections, "General", "Stretch", "100")) * stretch))
        if "Distortion" not in sections:
            order.append("Distortion")
        on = get(sections, "Distortion", "On", "0") == "1"
        put(sections, "Distortion", "On", 1)
        if not on:
            put(sections, "Distortion", "Clipping", 0)
        put(sections, "Distortion", "Bits", max(bits, int(get(sections, "Distortion", "Bits", "0")) if on else 0))
        put(sections, "Distortion", "Rate", max(rate, int(get(sections, "Distortion", "Rate", "0")) if on else 0))
        write(os.path.join(out, name + ".ds"), order, sections)
    print(kit, len(pieces))

# name, source patch, tuning (semitones), stretch (decay)
make("dmx", 4, 1, [
    ("bass1", "linn/linn_kik", -1.0, 1.15), ("bass2", "linn/linn_kik", 1.0, 0.9), ("bass3", "electro/K_Linn", -2.0, 1.0),
    ("snare1", "linn/linn_snr", 0.0, 1.0), ("snare2", "linn/linn_snr", -2.0, 1.25), ("snare3", "electro/S_Linn", 0.0, 1.0),
    ("rimshot", "linn/linn_stk", 0.0, 1.0), ("claps", "linn/linn_clp", 0.0, 1.0),
    ("hihat_closed", "tr909/Hat-c", -1.0, 0.9), ("hihat_accent", "tr909/Hat-c2", -1.0, 1.0),
    ("hihat_open", "tr909/Hat-o", -1.0, 1.0),
    ("tom1", "linn/linn_tom1", 0.0, 1.0), ("tom2", "linn/linn_tom2", 0.0, 1.0), ("tom3", "linn/linn_tom3", 0.0, 1.0),
    ("tom4", "linn/linn_tom4", 0.0, 1.0), ("tom5", "linn/linn_tom5", 0.0, 1.0), ("tom6", "linn/linn_tom5", -3.0, 1.2),
    ("ride", "tr808/Ride", -2.0, 1.2), ("crash", "cr8000/Cymbal", -1.0, 1.5),
    ("tamb", "linn/linn_tamb", 0.0, 1.0), ("shaker", "cr78/Maracas", 0.0, 1.0),
])
make("tr707", 5, 1, [
    ("bass1", "tr909/Kick_1", 0.0, 0.85), ("bass2", "tr909/Kick", 1.0, 0.8),
    ("snare1", "tr909/Snare", 0.0, 0.9), ("snare2", "tr909/Snare2", 1.0, 0.85),
    ("tom_low", "linn/linn_tom4", -1.0, 0.8), ("tom_mid", "linn/linn_tom2", -1.0, 0.8), ("tom_high", "linn/linn_tom1", 0.0, 0.8),
    ("rimshot", "tr808/Rimshot", 0.0, 1.0), ("cowbell", "tr808/Cowbell", 1.0, 0.8),
    ("handclap", "tr909/Clap", 0.0, 0.9), ("tambourine", "linn/linn_tamb", 1.0, 0.9),
    ("hihat_closed", "tr909/Hat-c", 0.0, 0.7), ("hihat_open", "tr909/Hat-o", 0.0, 0.8),
    ("crash", "cr8000/Cymbal", 0.0, 1.2), ("ride", "tr808/Ride", 0.0, 1.0),
])
