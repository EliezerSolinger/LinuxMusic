# Bibliotecas de sons para os players de samples (GIG, SF2, PAT)

Pesquisa feita em 17/11/2026. Objetivo: escolher conjuntos de sons que possam
ir **dentro** do LinuxMusic (GPL-2.0-or-later) para o GIG Player, o Sf2Player
e o Patman, cada um com uma biblioteca embutida e um botão para carregar
arquivos do usuário. Este documento é só pesquisa: nada foi baixado para o
projeto e nenhum código foi alterado.

A coluna "Licença lida na fonte?" diz se o texto da licença (ou a declaração
oficial do autor) foi efetivamente lido no site/arquivo do autor, ou se a
informação veio de terceiros (Debian, fóruns, catálogos).

> Aviso: isto não é parecer jurídico. Onde há dúvida, o item está marcado.

---

## 1. Situação atual no repositório

- `data/samples/soundfonts/FluidR3Mono_GM.sf3` (14,5 MB) com
  `FluidR3Mono_License.txt` (MIT; créditos a Frank Wen, Michael Cowgill,
  Ethan Winer e Michael Schorsch). É o único banco GM que enviamos.
- `data/avldrums/*.sf2` — kits de bateria AVL (já tratados em outro trabalho).
- Nenhum arquivo `.gig` nem `.pat` é distribuído.
- `data/samples/CMakeLists.txt` instala `*.sf2;*.sf3;*.txt` (entre outros),
  mas não `*.gig` nem `*.pat` — será preciso incluir essas extensões.

## 2. Critério de licença usado

Aceitas: domínio público / CC0, CC-BY, CC-BY-SA, MIT, GPL (com ou sem
exceção para composições) e afins. Todas permitem redistribuição, inclusive
comercial, que é o que a GPL exige de quem redistribui o programa.

Rejeitadas: qualquer cláusula "não comercial" (CC-NC, CC Sampling Plus,
"uso pessoal"), "não redistribuir", "não modificar", "não oferecer como
instrumento de sampler", ou licença ausente/indefinida.

Observação sobre GPL-3: bancos sob GPL-3-or-later (FreePats GM) são dados
distribuídos ao lado do programa (agregação), não código ligado a ele; o
risco é baixo, mas fica registrado.

---

## 3. SoundFont 2 (Sf2Player — `.sf2` / `.sf3`)

| Conjunto | Licença (onde está escrita) | Tamanho | Cobertura | Licença lida na fonte? |
|---|---|---|---|---|
| **FluidR3Mono_GM** (já enviado) | MIT — `FluidR3Mono_License.txt` no próprio pacote | 14,5 MB (.sf3) | GM completo (128 + baterias) | Sim (arquivo no repo) |
| **FluidR3_GM** (estéreo original) | MIT — README/COPYING do pacote; confirmado no `copyright` do Debian (`fluid-soundfont`) | ~142 MB (.sf2) | GM completo | Parcial: lido via Debian |
| **MuseScore_General** (v0.2, base FluidR3Mono) | MIT — `MuseScore_General_License.md` no espelho oficial | 38 MB (.sf3) / 206 MB (.sf2) | GM completo + extras; lista de origens em `MuseScore_General_Sample_Sources.csv` | Sim |
| **GeneralUser GS** 2.0.3 (S. Christian Collins) | "GeneralUser GS License v2.0" — `documentation/LICENSE.txt` no GitHub do autor | 32,3 MB (.sf2, medido) | 261 presets GM/GS + 13 kits de bateria | Sim |
| **TimGM6mb** (Tim Brechbill) | GPL-2 — `copyright` do Debian (`timgm6mb-soundfont`) e manual do MuseScore | ~6 MB | GM completo, qualidade modesta | Não (só Debian/MuseScore; site original fora do ar) |
| **FreePats GM** 2022-10-26 | GPL-3-or-later com exceção para composições — página do conjunto | 227 MiB compactado / ~307 MiB (.sf2) | GM **incompleto** (45 melódicos + 43 percussões) | Sim (declaração na página) |
| **FreePats — instrumentos avulsos** | CC0 1.0 (maioria) ou CC-BY — página de cada instrumento | 2–130 MiB cada | ver abaixo | Sim (declaração na página) |
| **Salamander Grand Piano** V3 (Alexander Holm) | CC-BY 3.0 — página da FreePats e README do repositório sfzinstruments | 296 MiB (.sf2) | Piano de cauda Yamaha C5, muitas camadas | Sim |
| **YDP Grand Piano** | CC-BY 3.0 — página de piano da FreePats | 36 MiB (.sf2) | Piano | Sim |

FreePats avulsos verificados (tamanho compactado; entre parênteses o valor que
a página dá em seguida, que deduzo ser o tamanho descompactado):

- Upright Piano KW (CC0): completo 27 MiB; "small" 5,8 MiB; "small bright" 6,0 MiB.
- Spanish Classical Guitar (CC0): 9,5 MiB (19 MiB).
- Clean Electric Bass, Pick/Finger (CC0): 2,2 MiB (3,4) / 2,5 MiB (4,0).
- FSBS Electric Guitar Clean #1 / #2 Jazz / Direct (CC0): versões "small" de 5–6 MiB; completas de 60–129 MiB.
- Synth Strings #1 / #2 (CC0): 4,2 MiB / 7,4 MiB.
- Church Organ Emulation (CC0): 13 MiB.
- MuldjordKit, bateria acústica (CC-BY 4.0): 53 MiB (209 MiB).
- FreePats GM Percussion (GPL-3+ c/ exceção): 15 MiB.

Notas de qualidade:
- GeneralUser GS é o melhor custo/benefício do grupo: soa próximo de bancos 2–3
  vezes maiores, mas depende de um sintetizador que siga bem a especificação
  (FluidSynth atende). O autor avisa que não consegue garantir a origem de
  todas as amostras (algumas vieram de outros bancos gratuitos) e pede para
  ser avisado de qualquer amostra problemática; também pede que não se faça
  link direto para os arquivos dele. A licença permite uso em software e
  modificação, mas não trata de redistribuição de forma explícita. Risco
  baixo, mas registrado.
- TimGM6mb: pequeno e completo, mas a procedência das amostras só é afirmada
  numa frase do autor reproduzida pelo Debian.
- FreePats tem a política de procedência mais rígida de todas (só material
  original, de origem conhecida, compatível com DFSG).

## 4. GigaSampler / GigaStudio (GIG Player — `.gig`)

**Conclusão principal: não encontrei nenhum `.gig` pronto com licença livre
verificada.** Os candidatos pesquisados:

| Conjunto | Licença | Tamanho | Situação |
|---|---|---|---|
| Maestro Concert Grand v2 (Mats Helgesson), linuxsampler.org | Licença própria: todos os direitos reservados, redistribuição só como presente, sem venda, sem modificação, publicar exige contato | 337 MB (.rar) / 932 MB | **Rejeitado** (licença lida na fonte) |
| London Philharmonia (tuba, violino), linuxsampler.org | Marcado "PENDING" no site; a Philharmonia proíbe oferecer as amostras "como estão", como amostras ou instrumento de sampler | 3–42 MB cada | **Rejeitado** |
| Mellotron "Taijiguy GigaTron", linuxsampler.org | Descrito pelo próprio site como grátis, mas não livre | 299 MB | **Rejeitado** |
| Jon Griffin free gigasamples / Cuban samples | Nenhuma licença encontrada; página fora do ar (404) | — | **Rejeitado** (sem licença) |
| G-Town Church (musical-artifacts.com) | Não foi possível ler (403) | — | Não verificado |
| Salamander Grand Piano em `.gig` | Não há versão oficial; existem conversões feitas por usuários | — | Possível conversão própria (CC-BY 3.0 permite) |

Caminho recomendado: **gerar nossos próprios `.gig`** a partir de amostras
WAV livres, com a ferramenta `wav2gig` da libgig (adicionada na versão 4.4.0,
jan/2024; cria um `.gig` a partir de uma pasta de WAVs) e ajuste fino no
gigedit. Fontes WAV livres verificadas:

| Fonte WAV | Licença (onde) | Tamanho | Cobertura |
|---|---|---|---|
| **VSCO-2 Community Edition** (Versilian) | CC0 — página oficial e repositório | ~3 GB de WAV (~3.000 amostras); v1.1.0 | Orquestra de câmara: cordas, metais, madeiras, percussão |
| **VCSL** (Versilian Community Sample Library) | CC0 — README e licença do repositório | 20–75 MB por instrumento (WAV); v1.2.2-RC | Teclados, cordofones, idiofones, membranofones, TX81Z |
| **FreePats Upright Piano KW** (SFZ+WAV) | CC0 | small: 4,3 MiB; completo: 69 MiB | Piano de armário |
| **FreePats Spanish Guitar / Electric Bass / Synth Strings / Church Organ** (SFZ+WAV) | CC0 | 4–19 MiB cada | Violão, baixo, cordas sintéticas, órgão |
| **Salamander Grand Piano V3** (SFZ+WAV 44,1 kHz/16 bits) | CC-BY 3.0 | 394 MiB compactado | Piano de cauda |
| **FreePats MuldjordKit** (SFZ+WAV) | CC-BY 4.0 | 223 MiB (380 MiB) | Bateria |

Observação: o `.gig` guarda PCM sem compressão, então o tamanho final fica
próximo do tamanho descompactado dos WAVs.

## 5. Gravis UltraSound (Patman — `.pat`)

| Conjunto | Licença | Tamanho | Situação |
|---|---|---|---|
| **FreePats (conjunto GUS antigo, 20060219)** | GPL-2-or-later com exceção para composições — `copyright` do Debian (`freepats`); distros listam "GPL"/"GPLv2+" | ~25 MB | **Aceito.** Único conjunto GUS livre conhecido; qualidade limitada e faltam instrumentos. O site atual da FreePats não oferece mais o download; os pacotes Debian/Ubuntu/MacPorts são os espelhos confiáveis. Licença lida via Debian, não no site original. |
| **Eawpats** (Eric A. Welsh) | "Fameware" informal; parte dos patches veio de espelhos com aviso de proibição de uso comercial | — | **Rejeitado** (fonte: thread debian-legal de 2002) |

Caminho recomendado para ampliar: **converter bancos livres para `.pat`**:

- `unsf` (github.com/psi29a/unsf) converte um `.sf2` em um `.pat` por
  instrumento. Gera o formato GUS "estendido"; players que não conhecem as
  extensões ainda tocam. O resultado cresce muito (exemplo citado: FluidR3_GM
  de 142 MB vira quase 2 GB de patches).
- `sfz2pat` (pacote `soundfont-utils`) converte um `.sfz` em `.pat`;
  interpreta só uma pequena parte do SFZ, reduz 24 bits para 16 bits e exige
  um patch por peça de bateria. O autor chama de experimento.
- **Pendência técnica:** testar se o carregador do Patman
  (`plugins/Patman/Patman.cpp`) aceita os `.pat` estendidos gerados pelo unsf.
- Licenças que permitem conversão: CC0 (FreePats avulsos, VSCO/VCSL), MIT
  (FluidR3, MuseScore_General), CC-BY (com aviso de modificação) e GeneralUser
  GS (permite modificar).

## 6. Rejeitados ou não verificados (resumo)

| Conjunto | Motivo |
|---|---|
| Sonatina Symphonic Orchestra (SFZ) | CC Sampling Plus 1.0 (aposentada pela Creative Commons): redistribuir a obra inteira só para fins não comerciais. Incompatível com empacotar dentro de um programa GPL. Lido no README do repositório (peastman/sso) e na página da licença. |
| Virtual Playing Orchestra | Mistura de fontes; inclui amostras da Philharmonia, cujos termos proíbem disponibilizá-las como instrumento de sampler. Também contém SSO (Sampling Plus). O autor pede que não seja reempacotada para venda. |
| Arachno SoundFont | O site do autor não mostra licença; o catálogo Polyphone indica "uso pessoal" com restrições de compartilhamento; contém sons de sintetizadores comerciais. |
| Maestro Concert Grand, Philharmonia `.gig`, Taijiguy Mellotron | Ver seção 4. |
| Eawpats | Ver seção 5. |
| Karoryfer Samples | A loja não mostra licença na página inicial; licença dos pacotes gratuitos **não verificada**. Rever antes de considerar. |
| SGM, Musyng Kite e outros bancos GM populares | Não pesquisados a fundo; historicamente com procedência duvidosa. Não usar sem verificação. |

## 7. Conjuntos disponíveis em mais de um dos nossos formatos

- **FreePats (novo)**: SF2 e SFZ (WAV/FLAC). SFZ+WAV → `.gig` (wav2gig) e
  → `.pat` (sfz2pat); SF2 → `.pat` (unsf). É a melhor base comum para os três
  players, com licenças CC0/CC-BY/GPL claras.
- **FreePats (antigo)**: `.pat` nativo; o FreePats GM novo é o sucessor em SF2.
- **Salamander Grand Piano**: SF2 (FreePats) e SFZ+WAV/FLAC; `.gig` só por
  conversão.
- **VSCO-2 CE / VCSL**: WAV e SFZ; podem virar `.gig` e `.pat`. Não há SF2
  oficial (a própria Versilian avisa que SFZ e SF2 não são compatíveis).
- **FluidR3 / MuseScore_General / GeneralUser GS**: só SF2/SF3; podem virar
  `.pat` com unsf.

## 8. Propostas de biblioteca

Tamanhos aproximados, somando os arquivos instalados (descompactados quando a
página informa).

### 8.1 Sf2Player

**Mínima (~50 MB novos + 14,5 MB já existentes):**
- FluidR3Mono_GM.sf3 (já enviado) — 14,5 MB, MIT.
- GeneralUser GS 2.0.3 — 32,3 MB, licença própria permissiva.
- FreePats Upright Piano KW small — ~6 MB, CC0.
- FreePats Spanish Classical Guitar — ~9,5 MB, CC0.
- FreePats Clean Electric Bass Finger — ~2,5 MB, CC0.

**Opcional (download à parte, ~1,1 GB):**
- MuseScore_General.sf3 — 38 MB, MIT.
- FluidR3_GM.sf2 estéreo — ~142 MB, MIT.
- FreePats GM SF2 — ~307 MB, GPL-3+ c/ exceção.
- Salamander Grand Piano SF2 — 296 MB, CC-BY 3.0.
- YDP Grand Piano — 36 MB, CC-BY 3.0.
- Upright Piano KW completo — 27 MB, CC0.
- MuldjordKit SF2 — 53 MB, CC-BY 4.0.
- FSBS Electric Guitar Clean #2 Jazz — 66 MB, CC0.
- Synth Strings #1 e #2, Church Organ — ~25 MB, CC0.
- TimGM6mb — ~6 MB, GPL-2 (alternativa leve, se quisermos).

### 8.2 GIG Player (tudo gerado por nós com wav2gig)

**Mínima (~60–70 MB):**
- Upright Piano KW small (CC0), Spanish Guitar (CC0, ~19 MB), Electric Bass
  Finger (CC0, ~6 MB), Synth Strings #1 (CC0, ~6 MB), Church Organ (CC0,
  ~13 MB), e 1–2 instrumentos curtos da VCSL (CC0, 20–40 MB).

**Opcional (~2–4 GB):**
- Upright Piano KW completo (~69 MB).
- Salamander Grand Piano 44,1 kHz/16 bits (CC-BY 3.0; algumas centenas de MB
  a ~1 GB conforme as camadas mantidas).
- VSCO-2 CE em seções (CC0; até ~3 GB se completo).
- MuldjordKit (CC-BY 4.0, ~380 MB).

### 8.3 Patman

**Mínima (~25–40 MB):**
- FreePats GUS 20060219 (~25 MB, GPL-2+ c/ exceção) — nativo, sem conversão.
- Opcionalmente, 2–3 instrumentos FreePats CC0 pequenos convertidos com
  sfz2pat/unsf (piano small, baixo, cordas sintéticas; ~10–15 MB), após o
  teste de compatibilidade do Patman.

**Opcional (~0,5–2 GB):**
- GM completo convertido com unsf a partir de GeneralUser GS ou FluidR3_GM
  (o resultado fica muitas vezes maior que o `.sf2`; FluidR3_GM chega a ~2 GB).
  Preferir GeneralUser (menor) ou converter só uma seleção de programas.

### 8.4 Empacotamento

- A biblioteca mínima pode ir no pacote/instalador. A opcional deve ser um
  download sob demanda (com hash SHA-256 conferido), e não ficar no
  repositório git.
- Cada conjunto em sua própria pasta, com a licença original junto.

## 9. Como registrar créditos e atribuição

1. **Licença original sempre junto**: copiar sem alterar o arquivo de licença
   e o README de cada conjunto para a pasta dele (como já fazemos com
   `FluidR3Mono_License.txt`). MIT, GPL e as CC-BY exigem manter os avisos.
2. **Arquivo central legível por máquina**, por exemplo
   `data/samples/CREDITOS` no formato `copyright` do Debian (DEP-5) ou um
   JSON, com um registro por conjunto: nome, versão, autores, identificador
   SPDX (`CC0-1.0`, `CC-BY-3.0`, `CC-BY-4.0`, `MIT`, `GPL-2.0-or-later`,
   `GPL-3.0-or-later` + `LicenseRef-FreePats-exception`,
   `LicenseRef-GeneralUser-GS-2.0`), URL de origem, data do download, SHA-256
   e "modificações feitas".
3. **Conversões (`.gig` e `.pat`)**: registrar que é obra derivada ("convertido
   pelo LinuxMusic a partir de X, versão Y, com a ferramenta Z"). CC-BY exige
   indicar que houve modificação; a exceção da FreePats pode ser mantida.
4. **Na interface**: o seletor de biblioteca de cada player mostra autor e
   licença do item escolhido, e a janela "Sobre" ganha uma aba "Sons" gerada a
   partir do arquivo central.
5. CC0 não exige crédito, mas creditar mesmo assim (Versilian, FreePats).

---

## 10. Fontes

- FluidR3Mono: arquivo `data/samples/soundfonts/FluidR3Mono_License.txt` do repositório.
- FluidR3 (Debian): https://metadata.ftp-master.debian.org/changelogs/main/f/fluid-soundfont/stable_copyright
- MuseScore_General (arquivos e licença): https://ftp.osuosl.org/pub/musescore/soundfont/MuseScore_General/ e https://ftp.osuosl.org/pub/musescore/soundfont/MuseScore_General/MuseScore_General_License.md
- GeneralUser GS: https://github.com/mrbumpy409/GeneralUser-GS e https://github.com/mrbumpy409/GeneralUser-GS/blob/main/documentation/LICENSE.txt ; resumo: https://scancode-licensedb.aboutcode.org/generaluser-gs-2.0.html
- TimGM6mb (Debian): https://metadata.ftp-master.debian.org/changelogs/main/t/timgm6mb-soundfont/stable_copyright ; MuseScore: https://musescore.org/en/handbook/3/soundfonts-and-sfz-files
- FreePats: https://freepats.zenvoid.org/ ; GM: https://freepats.zenvoid.org/SoundSets/general-midi.html ; pianos: https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html ; política: https://freepats.zenvoid.org/about.html ; violão: https://freepats.zenvoid.org/Guitar/acoustic-guitar.html ; guitarra: https://freepats.zenvoid.org/ElectricGuitar/clean-electric-guitar.html ; baixo: https://freepats.zenvoid.org/ElectricGuitar/clean-electric-bass.html ; cordas sintéticas: https://freepats.zenvoid.org/Synthesizer/synth-strings.html ; órgão: https://freepats.zenvoid.org/Organ/pipe-organ.html ; bateria: https://freepats.zenvoid.org/Percussion/acoustic-drum-kit.html
- FreePats GUS antigo (Debian): https://metadata.ftp-master.debian.org/changelogs/main/f/freepats/stable_copyright ; https://tracker.debian.org/pkg/freepats ; https://pkgsrc.se/audio/freepats
- Salamander (SFZ): https://github.com/sfzinstruments/SalamanderGrandPiano ; fórum LinuxSampler: https://bb.linuxsampler.org/viewtopic.php?p=3103
- LinuxSampler instrumentos: https://www.linuxsampler.org/instruments.html ; licença do Maestro: http://download.linuxsampler.org/instruments/pianos/maestro_concert_grand_v2_license.txt
- libgig (ferramentas) e wav2gig: https://www.linuxsampler.org/libgig/ ; https://svn.linuxsampler.org/svn/libgig/trunk/NEWS
- Philharmonia: https://philharmonia.co.uk/resources/sound-samples/
- VSCO-2 CE: https://versilian-studios.com/vsco-community/ ; https://github.com/sgossner/VSCO-2-CE
- VCSL: https://github.com/sgossner/VCSL
- Sonatina: https://github.com/peastman/sso ; licença: https://creativecommons.org/licenses/sampling+/1.0/
- Virtual Playing Orchestra: https://virtualplaying.com/virtual-playing-orchestra/
- Arachno: https://www.arachnosoft.com/main/soundfont.php ; https://www.polyphone.io/en/soundfonts/instrument-sets/255-arachno-soundfont
- Eawpats (debian-legal): https://lists.debian.org/debian-legal-0209/msg00137.html
- Karoryfer: https://shop.karoryfer.com/
- Lista Linux Audio: https://wiki.linuxaudio.org/wiki/free_audio_data
- unsf: https://github.com/psi29a/unsf ; https://www.mankier.com/1/unsf ; sfz2pat: https://www.mankier.com/1/sfz2pat
