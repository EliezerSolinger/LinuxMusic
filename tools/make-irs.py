#!/usr/bin/env python3
"""
make-irs.py - the impulse responses that come with the Amp & IR plugin (data/ir), made from models, so that they
are free of anyone's rights (CC0, as LinuxMusic's own work): guitar cabinets, an acoustic guitar's body, and rooms.
They are models, not measurements of real cabinets or halls.

Run from the repository's root: python3 tools/make-irs.py
"""
import math
import os
import random
import struct
import wave

RATE = 48000


def biquad(kind, f0, q, gain_db=0.0):
    """RBJ biquad coefficients (b0, b1, b2, a1, a2), normalized"""
    a = 10 ** (gain_db / 40)
    w0 = 2 * math.pi * f0 / RATE
    alpha = math.sin(w0) / (2 * q)
    c = math.cos(w0)
    if kind == "peak":
        b = [1 + alpha * a, -2 * c, 1 - alpha * a]
        d = [1 + alpha / a, -2 * c, 1 - alpha / a]
    elif kind == "lowpass":
        b = [(1 - c) / 2, 1 - c, (1 - c) / 2]
        d = [1 + alpha, -2 * c, 1 - alpha]
    elif kind == "highpass":
        b = [(1 + c) / 2, -(1 + c), (1 + c) / 2]
        d = [1 + alpha, -2 * c, 1 - alpha]
    else:
        raise ValueError(kind)
    return [x / d[0] for x in b] + [d[1] / d[0], d[2] / d[0]]


def filtered(signal, sections):
    out = list(signal)
    for b0, b1, b2, a1, a2 in sections:
        x1 = x2 = y1 = y2 = 0.0
        for n, x in enumerate(out):
            y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2
            x2, x1, y2, y1 = x1, x, y1, y
            out[n] = y
    return out


def impulse(seconds):
    signal = [0.0] * int(seconds * RATE)
    signal[0] = 1.0
    return signal


def write(path, channels):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    peak = max(max(abs(x) for x in ch) for ch in channels) or 1.0
    frames = len(channels[0])
    with wave.open(path, "wb") as out:
        out.setnchannels(len(channels))
        out.setsampwidth(3)
        out.setframerate(RATE)
        data = bytearray()
        for n in range(frames):
            for ch in channels:
                v = int(round(ch[n] / peak * 0.9 * 8388607))
                data += struct.pack("<i", v)[:3]
        out.writeframes(bytes(data))


def fade(signal, seconds):
    count = int(seconds * RATE)
    for i in range(count):
        signal[-1 - i] *= i / count
    return signal


def cabinet(name, lows, peaks, top):
    """A speaker in a cabinet: a high pass at the cabinet's resonance, a low pass at the cone's top, its peaks"""
    sections = [biquad("highpass", lows, 0.9)]
    sections += [biquad("peak", f, q, g) for f, q, g in peaks]
    sections += [biquad("lowpass", top, 0.8), biquad("lowpass", top * 1.4, 0.7)]
    write(f"data/samples/impulse-responses/Guitar cabinets/{name}.wav", [fade(filtered(impulse(0.2), sections), 0.05)])


def body(name, modes):
    """An acoustic guitar's body: its air and top modes ringing, over a gentle high roll-off"""
    signal = impulse(0.4)
    out = [0.0] * len(signal)
    random.seed(name)
    for f, q, g in modes:
        ring = filtered(signal, [biquad("peak", f, q, g)])
        out = [a + b - s for a, b, s in zip(out, ring, signal)]
    out = [a + s for a, s in zip(out, signal)]
    out = filtered(out, [biquad("lowpass", 7000, 0.7), biquad("highpass", 60, 0.7)])
    write(f"data/samples/impulse-responses/Acoustic guitar/{name}.wav", [fade(out, 0.1)])


def room(name, seconds, early, damping, seed):
    """A room: early reflections, then a dense tail of noise decaying to -60 dB in "seconds", the highs faster;
    each side its own noise (decorrelated)"""
    random.seed(seed)
    channels = []
    frames = int(seconds * 1.2 * RATE)
    for side in range(2):
        tail = [0.0] * frames
        for n in range(frames):
            t = n / RATE
            tail[n] = random.gauss(0, 1) * 10 ** (-3 * t / seconds)
        # The highs die faster: low passes that close over time, in pieces
        pieces = []
        step = int(0.05 * RATE)
        for start in range(0, frames, step):
            t = start / RATE
            cutoff = max(1500.0, damping * (1 - t / (seconds * 1.2)) + 1500.0)
            pieces += filtered(tail[start:start + step], [biquad("lowpass", min(cutoff, 18000.0), 0.7)])
        tail = pieces[:frames]
        for delay, gain in early:
            index = int(delay * RATE * (1.0 + 0.03 * side))
            if index < frames:
                tail[index] += gain * 20
        tail[0] += 25  # the direct sound's place: the start
        channels.append(fade(tail, 0.2))
    write(f"data/samples/impulse-responses/Rooms/{name}.wav", channels)


cabinet("4x12 closed back", 80, [(110, 1.5, 4), (1800, 1.2, 3), (3200, 2.0, 5), (6000, 1.0, -8)], 5200)
cabinet("2x12 open back", 95, [(140, 1.2, 2), (1200, 1.0, 2), (2600, 1.5, 4)], 6000)
cabinet("1x12 combo", 110, [(160, 1.0, 3), (2200, 1.6, 4), (4500, 2.0, -4)], 5500)
cabinet("1x10 small", 140, [(220, 1.0, 3), (1600, 1.2, 3), (3000, 1.4, 2)], 4800)
body("Dreadnought", [(98, 8, 12), (196, 6, 9), (230, 5, 6), (400, 4, 4), (780, 3, 3)])
body("Classical", [(90, 8, 10), (185, 6, 8), (290, 5, 5), (520, 4, 3)])
room("Small room", 0.5, [(0.004, 0.6), (0.007, 0.5), (0.011, 0.4), (0.017, 0.3)], 9000, 1)
room("Studio live room", 0.9, [(0.006, 0.6), (0.010, 0.5), (0.015, 0.45), (0.023, 0.35)], 10000, 2)
room("Hall", 2.2, [(0.012, 0.5), (0.019, 0.45), (0.031, 0.4), (0.047, 0.35)], 8000, 3)
room("Plate", 1.8, [(0.001, 0.3), (0.002, 0.3)], 14000, 4)
