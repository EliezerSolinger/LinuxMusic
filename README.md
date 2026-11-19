<div align="center">
	<h1>LinuxMusic</h1>
	<p>Free music production software for Linux</p>
</div>

LinuxMusic began as a fork of [LMMS](https://github.com/LMMS/lmms). It is now being rewritten with a new architecture.
The repository holds two programs:

- [`legado/`](legado/README.md) is the LMMS-based program, frozen at the tag `lmms-legado`. It builds and runs as before:

  ```sh
  git clone --recursive https://github.com/EliezerSolinger/LinuxMusic.git
  cd LinuxMusic
  cmake -S legado -B build -DCMAKE_BUILD_TYPE=RelWithDebInfo
  cmake --build build
  ./build/linuxmusic
  ```

- [`LinuxMusicBeta/`](LinuxMusicBeta/docs/arquitetura-nova.md) is the new program. It has only its design documents
  for now (in Portuguese). The code comes once the architecture is decided.

Licensed under the GNU General Public License v2 or later (see `LICENSE.txt`).
