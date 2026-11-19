#!/usr/bin/env python3
"""Gives a drum sound a sampler's character (the SP-1200's or the MPC60's), for the Drum Machines kits.

usage: character.py <sp1200|mpc60> <input> <output.flac> [pitch semitones]

SP-1200: sampled at 26.04 kHz (no anti-alias filter: what is above 13 kHz folds), 12 bits, played back held, step
by step (its DAC with no reconstruction filter: the images of the steps stay), a slight darkening (its output's
SSM2044 filters, open). MPC60: sampled at 40 kHz, 16 bits, a soft saturation and a gentle low pass at 15 kHz (its
output stage). A pitch (semitones) is applied as the samplers do: by playing at another rate.
"""
import math, struct, subprocess, sys, wave, tempfile, os

def decode(path, rate=44100):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-f", "s16le", "-ac", "1", "-ar", str(rate), "-"],
                         capture_output=True, check=True).stdout
    n = len(raw) // 2
    return [x / 32768.0 for x in struct.unpack("<%dh" % n, raw)]

def resample_linear(x, ratio):
    """x read at steps of 1/ratio (ratio = out rate / in rate), linear between samples"""
    out = []
    n = int(len(x) * ratio)
    for i in range(n):
        t = i / ratio
        j = int(t); f = t - j
        a = x[j] if j < len(x) else 0.0
        b = x[j + 1] if j + 1 < len(x) else 0.0
        out.append(a + (b - a) * f)
    return out

def quantize(x, bits):
    q = float(2 ** (bits - 1))
    return [max(-1.0, min(1.0, round(v * q) / q)) for v in x]

def hold(x, from_rate, to_rate):
    """zero-order hold: each sample held for its time at the output rate"""
    n = int(len(x) * to_rate / from_rate)
    return [x[min(len(x) - 1, int(i * from_rate / to_rate))] for i in range(n)]

def one_pole(x, cutoff, rate):
    a = math.exp(-2 * math.pi * cutoff / rate)
    y, out = 0.0, []
    for v in x:
        y = (1 - a) * v + a * y
        out.append(y)
    return out

def normalize(x, peak=0.89):
    m = max((abs(v) for v in x), default=0.0)
    return [v * peak / m for v in x] if m > 0 else x

def sp1200(x, rate, pitch):
    sampled = resample_linear(x, 26040.0 / rate * (2 ** (-pitch / 12.0)))
    sampled = quantize(normalize(sampled, 0.95), 12)
    held = hold(sampled, 26040.0, rate)
    return one_pole(held, 16000, rate)

def mpc60(x, rate, pitch):
    sampled = resample_linear(x, 40000.0 / rate * (2 ** (-pitch / 12.0)))
    sampled = quantize(normalize(sampled, 0.95), 16)
    back = resample_linear(sampled, rate / 40000.0)
    drive = 1.6
    saturated = [math.tanh(drive * v) / math.tanh(drive) for v in back]
    return one_pole(one_pole(saturated, 15000, rate), 15000, rate)

def main():
    kind, src, dst = sys.argv[1], sys.argv[2], sys.argv[3]
    pitch = float(sys.argv[4]) if len(sys.argv) > 4 else 0.0
    rate = 44100
    x = decode(src, rate)
    y = normalize((sp1200 if kind == "sp1200" else mpc60)(x, rate, pitch))
    # (a short fade at the end: no step at the last sample)
    fade = min(len(y), 64)
    for i in range(fade):
        y[len(y) - fade + i] *= 1 - (i + 1) / fade
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
        name = tmp.name
    with wave.open(name, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(rate)
        w.writeframes(struct.pack("<%dh" % len(y), *(int(max(-1, min(1, v)) * 32767) for v in y)))
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", name, "-c:a", "flac", dst], check=True)
    os.unlink(name)

main()
