<div align="center">
	<h1>LinuxMusic</h1>
	<p>Free music production software for Linux</p>
</div>

What is LinuxMusic?
--------------------

LinuxMusic is a fork of [LMMS](https://github.com/LMMS/lmms), the open-source digital audio workstation.
Its goal is to become a lean Linux DAW with REAPER-style live audio recording.

LinuxMusic targets Linux first. It also builds on macOS for development.

Roadmap
--------

The work is split into three long stages (details in [`docs/`](docs/README.md), in Portuguese):

1. Remove plugins and features that won't be used.
2. Rework the interface and usability, and lay the groundwork for the mixer and channel strips.
3. Implement a REAPER-style recording system (record arm, per-track inputs, monitoring, recording straight to disk).

Features
---------

* Song-Editor for arranging melodies, samples, patterns, and automation
* Pattern-Editor for creating beats and patterns
* An easy-to-use Piano-Roll for editing patterns and melodies
* A Mixer with unlimited mixer channels and arbitrary number of effects
* Many powerful instrument and effect-plugins out of the box
* Full user-defined track-based automation and computer-controlled automation sources
* Compatible with many standards such as SoundFont2, VST2 (instruments and effects), LADSPA, LV2, GUS Patches, and full MIDI support
* MIDI file importing and exporting

Building
---------

```sh
git clone --recursive https://github.com/EliezerSolinger/LinuxMusic.git
cd LinuxMusic
cmake -S . -B build -DCMAKE_BUILD_TYPE=RelWithDebInfo
cmake --build build
./build/linuxmusic
```

Build dependencies are the same as LMMS. See
[Compiling LMMS](https://github.com/LMMS/lmms/wiki/Compiling) and, for Ubuntu,
`.github/workflows/deps-ubuntu-24.04-gcc.txt`.

Credits and license
--------------------

LinuxMusic is built on the work of the LMMS developers and contributors (see `doc/AUTHORS` and
`doc/CONTRIBUTORS`). Like LMMS, it is licensed under the GNU General Public License v2 or later.
