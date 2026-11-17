#!/usr/bin/env python3
"""ab-compare.py <dir a> <dir b> [--noise FILE] [--limit DB]

Compares each pair of WAVs (same name in both directories): the largest and the RMS difference, in dBFS.
A project listed in the noise file (one name per line, '#' comments; ab-random.txt beside this script unless
another is given) renders differently on every run (it uses
random numbers) and is reported but not judged. Exits 1 if any other project differs by more than the limit
(default -120 dBFS: renders are the same on every run, bit for bit, but for ladspa-mbeq, whose FFTW plans are
measured at run time and differ around -138 dBFS), or if a file is missing or has another length.
"""
import array, math, os, struct, sys

def read(path):
    data = open(path, 'rb').read()
    pos, fmt, samples = 12, None, None
    while pos + 8 <= len(data):
        cid, size = data[pos:pos + 4], struct.unpack('<I', data[pos + 4:pos + 8])[0]
        body = data[pos + 8:pos + 8 + size]
        if cid == b'fmt ':
            fmt = struct.unpack('<HHIIHH', body[:16])
        elif cid == b'data' and fmt:
            if fmt[0] == 3 and fmt[5] == 32:
                samples = array.array('f', body)
            elif fmt[0] == 1 and fmt[5] == 16:
                samples = array.array('f', (s / 32768 for s in array.array('h', body)))
        pos += 8 + size + (size & 1)
    if samples is None:
        raise ValueError(f'{path}: not a 32-bit float or 16-bit WAV')
    return samples

def db(x):
    return -999.0 if x <= 0 else 20 * math.log10(x)

def main():
    args = sys.argv[1:]
    noise, limit = set(), -120.0
    noise_file = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'ab-random.txt')
    if '--noise' in args:
        i = args.index('--noise')
        noise_file = args[i + 1]
        del args[i:i + 2]
    if os.path.exists(noise_file):
        for line in open(noise_file):
            line = line.split('#')[0].strip()
            if line:
                noise.add(line)
    if '--limit' in args:
        i = args.index('--limit')
        limit = float(args[i + 1])
        del args[i:i + 2]
    if len(args) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    a, b = args
    bad = []
    for name in sorted(f for f in os.listdir(a) if f.endswith('.wav')):
        project = name[:-4]
        other = os.path.join(b, name)
        if not os.path.exists(other):
            print(f'{project:40s} missing in {b}')
            bad.append(project)
            continue
        x, y = read(os.path.join(a, name)), read(other)
        n = min(len(x), len(y))
        largest, total = 0.0, 0.0
        for i in range(n):
            d = abs(x[i] - y[i])
            total += d * d
            largest = max(largest, d)
        rms = math.sqrt(total / n) if n else 0.0
        note = ''
        if len(x) != len(y):
            note = f'  LENGTH {len(x)} vs {len(y)}'
        if project in noise:
            note += '  (random, not judged)'
        elif len(x) != len(y) or db(largest) > limit:
            note += '  DIFFERENT'
            bad.append(project)
        print(f'{project:40s} max {db(largest):8.1f} dB  rms {db(rms):8.1f} dB{note}')
    for name in sorted(f for f in os.listdir(b) if f.endswith('.wav')):
        if not os.path.exists(os.path.join(a, name)):
            print(f'{name[:-4]:40s} missing in {a}')
            bad.append(name[:-4])
    print('same' if not bad else f'{len(bad)} different: {" ".join(bad)}')
    return 1 if bad else 0

sys.exit(main())
