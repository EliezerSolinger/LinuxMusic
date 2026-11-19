#!/usr/bin/env python3
"""corrupt-projects.py <output dir> <projects>...

Writes damaged copies of each project, always the same ones (the random numbers are seeded with the project's
name): cut at 5, 20, 50, 80 and 97% of its length, with 20 bytes changed (four times), with numbers replaced by
nan, inf, huge, negative, empty and non-numeric values (eight times), and with an element removed (three times).
Loading any of them may fail, but must never crash the program (see corrupt-all.sh).
"""
import sys, os, random, re
out = sys.argv[1]
for path in sys.argv[2:]:
    name = os.path.splitext(os.path.basename(path))[0]
    data = open(path, 'rb').read()
    rng = random.Random(name)
    # Truncated
    for cut in (0.05, 0.2, 0.5, 0.8, 0.97):
        open(f'{out}/{name}-cut{int(cut*100)}.mmp', 'wb').write(data[:int(len(data) * cut)])
    # Bytes flipped
    for n in range(4):
        b = bytearray(data)
        for _ in range(20):
            i = rng.randrange(len(b))
            b[i] = rng.randrange(256)
        open(f'{out}/{name}-flip{n}.mmp', 'wb').write(bytes(b))
    # Values out of range in numeric attributes
    text = data.decode('utf-8', 'replace')
    attrs = list(re.finditer(r'(\w+)="(-?[0-9.]+)"', text))
    for n, bad in enumerate(['nan', 'inf', '-1e30', '1e30', '-2147483648', '99999999999', '', 'abc']):
        t = text
        for m in rng.sample(attrs, min(25, len(attrs))):
            t = t.replace(m.group(0), f'{m.group(1)}="{bad}"', 1)
        open(f'{out}/{name}-value{n}.mmp', 'w').write(t)
    # Elements removed
    elements = list(re.finditer(r'<(\w+)[ >]', text))
    for n in range(3):
        m = rng.choice(elements)
        tag = m.group(1)
        t = re.sub(rf'<{tag}\b[^>]*/>', '', text, count=1)
        open(f'{out}/{name}-noelem{n}.mmp', 'w').write(t)
