# Re-apply LinuxMusic branding to user-visible strings (code string literals, .ui, .ts).
# Run after merging from LMMS: python3 tools/rebrand.py . [files...]   (no files = whole tree)
import re, subprocess, pathlib, sys, os
root=sys.argv[1]; os.chdir(root)
files=sys.argv[2:] or subprocess.run(['git','ls-files','src','include','plugins','data/locale'],capture_output=True,text=True).stdout.split()
lit=re.compile(r'"(?:[^"\\\n]|\\.)*"'); word=re.compile(r'\bLMMS\b')
for f in files:
    if f.startswith('src/3rdparty'): continue
    p=pathlib.Path(f)
    if not p.is_file(): continue
    s=p.read_text(errors='surrogateescape')
    if f.endswith('.ts'): new=word.sub('LinuxMusic',s)
    elif f.endswith('.ui'):
        new=re.sub(r'(<string[^>]*>)(.*?)(</string>)', lambda m: m.group(1)+word.sub('LinuxMusic',m.group(2))+m.group(3), s, flags=re.S)
    elif f.endswith(('.cpp','.h')):
        out=[]
        for line in s.split('\n'):
            t=line.lstrip()
            if t.startswith(('//','*','/*','#include')): out.append(line); continue
            out.append(lit.sub(lambda m: word.sub('LinuxMusic',m.group(0)), line))
        new='\n'.join(out)
    else: continue
    if new!=s: p.write_text(new,errors='surrogateescape'); print('rebranded',f)
