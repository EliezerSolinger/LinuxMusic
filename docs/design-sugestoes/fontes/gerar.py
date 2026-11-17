#!/usr/bin/env python3
"""Gera as telas de sugestão de design (HTML) e as fotografa em PNG com o Chrome sem janela.
Uso: python3 gerar.py  (a partir desta pasta). As imagens vão para a pasta de cima."""
import os, subprocess, pathlib

AQUI = pathlib.Path(__file__).resolve().parent
SAIDA = AQUI.parent
CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

def pagina(nome, titulo, sub, mock, notas):
    itens = "".join(f'<div class="note"><div class="n">{i}</div><div>{t}</div></div>' for i, t in enumerate(notas, 1))
    html = f'''<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="base.css"></head>
<body><div class="page"><div class="mock">{mock}</div>
<div class="notes"><h1>{titulo}</h1><div class="sub">{sub}</div>{itens}</div></div></body></html>'''
    arquivo = AQUI / f"{nome}.html"
    arquivo.write_text(html, encoding="utf-8")
    subprocess.run([CHROME, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--window-size=1600,1000",
                    f"--screenshot={SAIDA / (nome + '.png')}", arquivo.as_uri()],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    print("ok", nome)

def b(n, x, y):
    return f'<div class="badge" style="left:{x}px;top:{y}px">{n}</div>'

# ---------------------------------------------------------------------------------------------------------------
# Peças reaproveitadas
def barra_superior():
    return '''
<div style="height:52px;background:var(--bg2);border-bottom:1px solid var(--line);display:flex;align-items:center;gap:8px;padding:0 12px">
  <div style="display:flex;gap:6px">
    <span class="btn ic">≡</span><span class="btn">Projeto ▾</span><span class="btn ic">↶</span><span class="btn ic">↷</span>
  </div>
  <div class="sep"></div>
  <div style="display:flex;gap:6px;align-items:center">
    <span class="btn ic">⏮</span><span class="btn ic">■</span><span class="btn ic on">▶</span>
    <span class="btn ic rec" style="color:var(--red);font-size:16px">●</span><span class="btn ic on" title="loop">⟲</span>
  </div>
  <div class="lcd" style="font-size:20px;letter-spacing:.04em">003.2.040 <span style="color:var(--tx3);font-size:12px">compasso</span></div>
  <div class="lcd" style="font-size:14px;color:var(--tx2)">0:05.214</div>
  <div class="sep"></div>
  <span class="btn">♩ 140,00</span><span class="btn">4/4</span><span class="btn on ic" title="metrônomo">◷</span>
  <span class="btn">Pré-roll 1</span>
  <div style="flex:1"></div>
  <div style="display:flex;flex-direction:column;gap:4px;width:110px">
    <div style="display:flex;justify-content:space-between;font-size:10px;color:var(--tx3)"><span>CPU</span><span>18%</span></div>
    <div class="hmeter"><i style="width:18%"></i></div>
    <div style="display:flex;justify-content:space-between;font-size:10px;color:var(--tx3)"><span>SAÍDA</span><span>-6 dB</span></div>
    <div class="hmeter"><i style="width:62%"></i></div>
  </div>
  <div class="sep"></div>
  <span class="btn on">Arranjo</span><span class="btn">Mixer</span><span class="btn">Sequências</span>
  <span class="btn ic">⚙</span>
</div>'''

def cabecalho_faixa(nome, cor, tipo, armado=False, mudo=False, solo=False, fx=0, nivel=60, altura=58, pasta=0, sel=False):
    fundo = "#2c333b" if sel else "var(--bg2)"
    return f'''
<div style="height:{altura}px;display:flex;border-bottom:1px solid var(--line);background:{fundo};padding-left:{pasta*14}px">
  <div style="width:5px;background:{cor}"></div>
  <div style="flex:1;padding:6px 8px;display:flex;flex-direction:column;gap:5px;min-width:0">
    <div style="display:flex;align-items:center;gap:5px">
      <span style="font-size:11px;color:var(--tx3);width:14px">{tipo}</span>
      <span style="flex:1;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis">{nome}</span>
      <span class="btn sm ic rec {'on' if armado else ''}">●</span>
      <span class="btn sm ic mute {'on' if mudo else ''}">M</span>
      <span class="btn sm ic solo {'on' if solo else ''}">S</span>
      <span class="btn sm {'on' if fx else ''}" style="min-width:30px">FX{(' '+str(fx)) if fx else ''}</span>
    </div>
    <div style="display:flex;align-items:center;gap:8px">
      <div class="slider" style="flex:1"><i style="width:{nivel}%"></i><b style="left:calc({nivel}% - 3px)"></b></div>
      <span style="font-size:10px;color:var(--tx2);width:38px;text-align:right">-{(100-nivel)//8:.0f},0 dB</span>
      <div class="slider" style="width:40px"><i style="width:50%;background:transparent"></i><b style="left:calc(50% - 3px)"></b></div>
    </div>
  </div>
  <div style="width:8px;padding:4px 2px"><div class="meter" style="height:100%"><i style="height:{nivel-10}%"></i></div></div>
</div>'''

def clipe(x, w, cor, nome, y=6, h=46, tipo="notas"):
    corpo = ""
    if tipo == "notas":
        corpo = "".join(f'<div style="position:absolute;left:{8+i*14}px;top:{16+((i*7)%22)}px;width:10px;height:3px;background:rgba(0,0,0,.45)"></div>' for i in range(max(1,(w-16)//14)))
    elif tipo == "onda":
        corpo = '<svg width="100%" height="30" style="position:absolute;top:14px" preserveAspectRatio="none" viewBox="0 0 100 30"><path d="M0 15 ' + " ".join(f"L{i} {15+((i*37)%13-6)*(1 if i%2 else -1)}" for i in range(1,100)) + '" stroke="rgba(0,0,0,.5)" fill="none" stroke-width=".8"/></svg>'
    return f'''<div style="position:absolute;left:{x}px;top:{y}px;width:{w}px;height:{h}px;border-radius:5px;background:{cor};overflow:hidden;box-shadow:inset 0 0 0 1px rgba(0,0,0,.35)">
<div style="font-size:10.5px;padding:2px 6px;color:#0b0d0f;font-weight:700;background:rgba(255,255,255,.18)">{nome}</div>{corpo}</div>'''

# ---------------------------------------------------------------------------------------------------------------
def tela_principal():
    faixas = [
        ("Bateria", "#e85a4f", "🥁", False, False, False, 2, 70, 0, False),
        ("Baixo", "#f0913a", "🎹", False, False, False, 1, 62, 0, False),
        ("Vocais", "#9b6be8", "📁", False, False, False, 0, 75, 0, False),
        ("Voz principal", "#9b6be8", "🎤", True, False, False, 3, 80, 1, True),
        ("Backing", "#9b6be8", "🎤", False, True, False, 0, 55, 1, False),
        ("Guitarra", "#3d8be8", "🎸", False, False, True, 2, 66, 0, False),
        ("Pad", "#3dc8d8", "🎹", False, False, False, 1, 48, 0, False),
    ]
    cab = "".join(cabecalho_faixa(f[0], f[1], f[2], armado=f[3], mudo=f[4], solo=f[5], fx=f[6], nivel=f[7],
        pasta=f[8], sel=f[9]) for f in faixas)
    clips = [
        [(0, 520, "#e8735f", "Batida A"), (530, 520, "#e8735f", "Batida B")],
        [(0, 1040, "#f2a457", "Baixo")],
        [],
        [(130, 380, "#b08cf0", "Take 3", "onda"), (560, 300, "#b08cf0", "Take 2", "onda")],
        [(130, 300, "#b08cf0", "Coro", "onda")],
        [(260, 520, "#6aa6ee", "Riff", "onda")],
        [(0, 1040, "#6ad6e2", "Pad")],
    ]
    linhas = ""
    for i, cl in enumerate(clips):
        h = 58
        conteudo = "".join(clipe(c[0], c[1], c[2], c[3], 6, 46, c[4] if len(c) > 4 else "notas") for c in cl)
        if i == 2:
            conteudo = '<div style="position:absolute;inset:18px 0 18px 0;background:repeating-linear-gradient(90deg,rgba(155,107,232,.18) 0 2px,transparent 2px 6px);border-radius:4px"></div>'
        linhas += f'<div style="height:{h}px;border-bottom:1px solid var(--line);position:relative">{conteudo}</div>'
    grade = "".join(f'<div style="position:absolute;top:0;bottom:0;left:{i*130}px;width:1px;background:{"#2f363d" if i%4 else "#3a424b"}"></div>' for i in range(9))
    regua = "".join(f'<div style="position:absolute;left:{i*130+4}px;top:4px;font-size:10px;color:var(--tx2)">{i+1}</div>' for i in range(9))
    mock = f'''
<div class="win">
  {barra_superior()}
  <div style="flex:1;display:flex;min-height:0">
    <!-- navegador -->
    <div style="width:44px;background:var(--bg2);border-right:1px solid var(--line);display:flex;flex-direction:column;align-items:center;gap:8px;padding-top:10px">
      <span class="btn ic on">⌕</span><span class="btn ic">♫</span><span class="btn ic">⚡</span><span class="btn ic">📁</span><span class="btn ic">★</span>
    </div>
    <div style="width:220px;background:var(--bg1);border-right:1px solid var(--line);padding:10px;display:flex;flex-direction:column;gap:8px">
      <div class="btn" style="justify-content:flex-start;color:var(--tx3)">⌕ Buscar instrumentos…</div>
      <div class="tag">Instrumentos</div>
      <div style="display:flex;flex-direction:column;gap:3px;font-size:12px">
        <div>▾ Sintetizadores</div><div style="padding-left:14px;color:var(--tx2)">▸ Subtrativos (3)</div><div style="padding-left:14px;color:var(--tx2)">▸ Tabela de onda (3)</div>
        <div style="padding-left:14px;color:var(--tx2)">▸ Completos (2)</div><div>▸ Samplers</div><div>▸ Baterias</div><div>▸ Chiptune</div><div>▸ Hosts de plugins</div>
      </div>
      <div class="tag" style="margin-top:6px">Recentes</div>
      <div style="font-size:12px;color:var(--tx2);display:flex;flex-direction:column;gap:3px"><div>TripleOscillator</div><div>AVL Drums</div><div>Sf2 Player</div></div>
    </div>
    <!-- arranjo -->
    <div style="flex:1;display:flex;flex-direction:column;min-width:0">
      <div style="height:36px;background:var(--bg2);border-bottom:1px solid var(--line);display:flex;align-items:center;gap:6px;padding:0 8px">
        <span class="btn sm on">✎ Desenhar</span><span class="btn sm">✂ Cortar</span><span class="btn sm">⬚ Selecionar</span><span class="btn sm">⌫ Apagar</span>
        <div class="sep"></div><span class="btn sm">Grade: 1/4 ▾</span><span class="btn sm on">Ímã</span><div style="flex:1"></div>
        <span class="btn sm">+ Faixa ▾</span><span class="btn sm">− ◯ +</span>
      </div>
      <div style="display:flex;flex:1;min-height:0">
        <div style="width:290px;border-right:1px solid var(--line)">
          <div style="height:26px;background:var(--bg2);border-bottom:1px solid var(--line);font-size:10px;color:var(--tx3);display:flex;align-items:center;padding-left:10px">7 FAIXAS · 1 GRAVANDO</div>
          {cab}
        </div>
        <div style="flex:1;position:relative;overflow:hidden">
          <div style="height:26px;background:var(--bg2);border-bottom:1px solid var(--line);position:relative">{regua}
            <div style="position:absolute;left:130px;width:390px;top:18px;height:6px;background:var(--green2);border-radius:2px;opacity:.8"></div></div>
          <div style="position:relative">{grade}{linhas}
            <div style="position:absolute;top:0;bottom:-200px;left:300px;width:2px;background:#fff;box-shadow:0 0 6px #fff"></div></div>
        </div>
      </div>
      <!-- editor de baixo -->
      <div style="height:30px;background:var(--bg2);border-top:1px solid var(--line);display:flex;align-items:center;gap:4px;padding:0 8px;font-size:12px">
        <span class="btn sm on">Piano roll</span><span class="btn sm">Automação</span><span class="btn sm">Mixer</span><div style="flex:1"></div><span style="color:var(--tx3)">▴ abrir</span>
      </div>
    </div>
    <!-- inspetor -->
    <div style="width:250px;background:var(--bg1);border-left:1px solid var(--line);padding:12px;display:flex;flex-direction:column;gap:10px">
      <div style="display:flex;align-items:center;gap:8px"><div style="width:10px;height:28px;background:#9b6be8;border-radius:2px"></div><div><div style="font-weight:700;font-size:14px">Voz principal</div><div style="font-size:11px;color:var(--tx3)">Faixa de áudio · em Vocais</div></div></div>
      <div class="tag">Entrada</div><div class="btn" style="justify-content:space-between">Entrada 1 (mono) <span>▾</span></div>
      <div style="display:flex;gap:6px"><span class="btn sm rec on">● Armada</span><span class="btn sm mon on">◉ Monitor</span><span class="btn sm">Lanes</span></div>
      <div class="tag">Volume · Pan</div>
      <div style="display:flex;gap:12px;align-items:center"><div class="knob"></div><div class="knob"></div><div style="font-size:11px;color:var(--tx2)">-2,4 dB · C</div></div>
      <div class="tag">Efeitos (3)</div>
      <div style="display:flex;flex-direction:column;gap:4px;font-size:12px">
        <div class="btn sm" style="justify-content:space-between">● EQ <span>⋯</span></div><div class="btn sm" style="justify-content:space-between">● Compressor <span>⋯</span></div><div class="btn sm" style="justify-content:space-between">● Reverb <span>⋯</span></div><div class="btn sm ghost">+ efeito</div>
      </div>
      <div class="tag">Envios</div><div class="btn sm" style="justify-content:space-between">→ Reverb bus <span>-12 dB</span></div>
      <div class="tag">Saída</div><div class="btn sm" style="justify-content:space-between">Vocais (grupo) <span>▾</span></div>
    </div>
  </div>
  <div style="height:22px;background:#0f1214;border-top:1px solid var(--line);font-size:11px;color:var(--tx3);display:flex;align-items:center;padding:0 10px;gap:18px">
    <span>Projeto salvo há 2 min</span><span>44,1 kHz · 256 amostras · 5,8 ms</span><span>Dica: Ctrl+arrastar copia o clipe</span>
  </div>
  {b(1,8,30)}{b(2,215,30)}{b(3,1030,30)}{b(4,30,98)}{b(5,268,64)}{b(6,262,250)}{b(7,880,190)}{b(8,1290,92)}{b(9,640,934)}{b(10,620,962)}
</div>'''
    notas = [
        "<b>Uma barra superior só.</b> Arquivo, desfazer e refazer à esquerda, sem a fileira de ícones soltos de hoje. O menu tradicional fica no botão ≡.",
        "<b>Transporte no centro, maior e com texto.</b> O tempo em compassos e em minutos, lado a lado. Andamento, compasso, metrônomo e pré-roll ficam ao alcance de um clique.",
        "<b>Medidores de CPU e saída legíveis</b>, com números, e as abas Arranjo, Mixer e Sequências no lugar do par 'Composição/Mixagem'.",
        "<b>Navegador com abas verticais</b> (buscar, instrumentos, efeitos, arquivos, favoritos) e <b>subcategorias</b>. Os recentes ficam no topo.",
        "<b>Uma barra de ferramentas por editor, com texto.</b> Hoje o arranjo e o piano roll empilham duas barras de ícones parecidos.",
        "<b>Cabeçalho de faixa no estilo REAPER</b>: cor, nome grande, gravar, mudo, solo e FX com contador; volume em régua horizontal, pan e medidor de altura inteira. Ver a imagem 2.",
        "<b>Clipes com nome e conteúdo</b> (notas ou forma de onda), cor da faixa, região de loop clara na régua e cursor de reprodução visível.",
        "<b>Inspetor à direita</b> para a faixa selecionada: entrada, armar e monitorar, volume, efeitos, envios e saída, sem abrir janelas flutuantes.",
        "<b>Editores encaixados embaixo</b>, em abas (piano roll, automação, mixer), que abrem e fecham sem cobrir o arranjo.",
        "<b>Barra de status</b> com o estado do projeto, a latência real e uma dica do contexto.",
    ]
    pagina("01-janela-principal", "1 · Janela principal",
           "Proposta de organização geral, com as cores atuais do tema. O mixer e a cadeia de efeitos ficam como estão.",
           mock, notas)

# ---------------------------------------------------------------------------------------------------------------
def recorte(img, x, y, w, h, escala=2, largura=1700):
    """Um pedaço da captura atual (de @p largura pixels), ampliado"""
    return (f'<div style="width:{w*escala}px;height:{h*escala}px;background:url({img}) -{x*escala}px -{y*escala}px / '
            f'{largura*escala}px auto no-repeat;border:1px solid #333;border-radius:6px;image-rendering:auto"></div>')

def tela_faixas():
    hoje = recorte("atual-arranjo.png", 226, 224, 340, 56, 2)
    estados = "".join([
        cabecalho_faixa("Bateria", "#e85a4f", "🥁", fx=2, nivel=70),
        cabecalho_faixa("Voz principal", "#9b6be8", "🎤", armado=True, fx=3, nivel=80, sel=True),
        cabecalho_faixa("Backing", "#9b6be8", "🎤", mudo=True, nivel=55, pasta=1),
        cabecalho_faixa("Guitarra", "#3d8be8", "🎸", solo=True, fx=1, nivel=66),
    ])
    expandida = f'''
<div style="height:120px;display:flex;border:1px solid var(--line);background:var(--bg2);border-radius:6px;overflow:hidden">
  <div style="width:5px;background:#9b6be8"></div>
  <div style="flex:1;padding:8px 10px;display:flex;flex-direction:column;gap:8px">
    <div style="display:flex;align-items:center;gap:6px"><span style="color:var(--tx3);font-size:11px">4</span><b style="font-size:14px;flex:1">Voz principal</b>
      <span class="btn sm ic rec on">●</span><span class="btn sm ic mon on">◉</span><span class="btn sm ic mute">M</span><span class="btn sm ic solo">S</span><span class="btn sm on">FX 3</span><span class="btn sm ic">⋯</span></div>
    <div style="display:flex;align-items:center;gap:8px"><span class="btn sm" style="width:130px;justify-content:space-between">Entrada 1 (mono) ▾</span><span class="btn sm" style="width:120px;justify-content:space-between">Saída: Vocais ▾</span><span class="btn sm">Lanes 3</span></div>
    <div style="display:flex;align-items:center;gap:8px"><span style="font-size:10px;color:var(--tx3);width:24px">VOL</span><div class="slider" style="flex:1"><i style="width:80%"></i><b style="left:calc(80% - 3px)"></b></div><span style="font-size:10px;width:40px">-2,4 dB</span>
      <span style="font-size:10px;color:var(--tx3)">PAN</span><div class="slider" style="width:70px"><b style="left:calc(50% - 3px)"></b></div><span style="font-size:10px">C</span></div>
  </div>
  <div style="width:12px;padding:5px 3px;display:flex;gap:1px"><div class="meter" style="flex:1"><i style="height:72%"></i></div><div class="meter" style="flex:1"><i style="height:64%"></i></div></div>
</div>'''
    mock = f'''
<div style="display:flex;gap:28px;height:100%">
  <div style="width:700px;display:flex;flex-direction:column;gap:14px">
    <div class="tag" style="font-size:12px;color:var(--tx2)">Hoje (ampliado 2×)</div>{hoje}
    <div style="color:var(--tx2);font-size:12.5px;line-height:1.5;max-width:680px">Botões de 14 px sem rótulo (engrenagem, M, S, R, "Omni"), knobs de volume e pan minúsculos, o número do canal do mixer num visor à parte e o nome espremido.</div>
    <div class="tag" style="font-size:12px;color:var(--tx2);margin-top:14px">Proposta: altura normal (58 px)</div>
    <div style="width:360px;border:1px solid var(--line);border-radius:6px;overflow:hidden;position:relative">{estados}
      {b(1,-12,-10)}{b(2,150,-10)}{b(3,330,62)}{b(4,140,40)}{b(5,350,150)}{b(7,-12,128)}</div>
    <div style="color:var(--tx2);font-size:12px">Estados: normal, armada para gravar (selecionada), muda e dentro de uma pasta, solo.</div>
  </div>
  <div style="flex:1;display:flex;flex-direction:column;gap:14px;position:relative">
    <div class="tag" style="font-size:12px;color:var(--tx2)">Proposta: altura grande (arrastar a borda)</div>
    <div style="width:470px;position:relative">{expandida}{b(6,440,-10)}</div>
    <div class="tag" style="font-size:12px;color:var(--tx2);margin-top:20px">Proposta: compacta (24 px, para muitas faixas)</div>
    <div style="width:470px;height:24px;display:flex;align-items:center;gap:6px;background:var(--bg2);border:1px solid var(--line);border-radius:6px;padding:0 6px;position:relative">
      <div style="width:4px;height:16px;background:#e85a4f"></div><b style="flex:1;font-size:12px">Bateria</b><span class="btn sm ic rec">●</span><span class="btn sm ic mute">M</span><span class="btn sm ic solo">S</span>
      <div class="hmeter" style="width:60px"><i style="width:70%"></i></div>{b(8,450,-10)}</div>
    <div class="tag" style="font-size:12px;color:var(--tx2);margin-top:20px">Clique direito no cabeçalho</div>
    <div style="width:250px;background:var(--bg3);border:1px solid var(--line2);border-radius:8px;padding:6px;font-size:12px;display:flex;flex-direction:column;gap:2px;position:relative">
      <div style="padding:5px 8px">Renomear…</div><div style="padding:5px 8px">Cor ▸</div><div style="padding:5px 8px">Ícone ▸</div>
      <div style="padding:5px 8px">Duplicar</div><div style="padding:5px 8px">Mover para pasta ▸</div><div style="padding:5px 8px">Congelar (renderizar)</div>
      <div style="height:1px;background:var(--line2);margin:4px 0"></div><div style="padding:5px 8px;color:var(--red)">Apagar faixa</div>{b(9,230,-10)}</div>
  </div>
</div>'''
    notas = [
        "<b>Barra de cor</b> à esquerda, a mesma cor dos clipes e do canal no mixer. O ícone diz o tipo (instrumento, áudio, pasta).",
        "<b>Nome grande e legível</b>, sem visor separado do número do canal. O canal aparece no inspetor e no mixer.",
        "<b>Botões de 20 px com estados por cor</b>: gravar vermelho, mudo laranja, solo amarelo, FX verde com o número de efeitos. Clique no FX abre os efeitos no inspetor.",
        "<b>Volume em régua horizontal</b> com o valor em dB e pan ao lado, no lugar dos knobs minúsculos. Arrastar com Shift faz o ajuste fino; duplo clique volta ao padrão.",
        "<b>Medidor de altura inteira</b> na borda direita, para ver de longe o que está tocando.",
        "<b>Altura grande</b>: entrada, saída, lanes e monitor aparecem sem abrir janela.",
        "<b>Pastas recuam</b> as faixas de dentro e somam a cor do grupo.",
        "<b>Altura compacta</b> para projetos grandes: só o essencial numa linha.",
        "<b>Menu de contexto curto</b>, com as ações destrutivas separadas e em vermelho.",
    ]
    pagina("02-cabecalho-de-faixa", "2 · Cabeçalho de faixa (estilo REAPER)",
           "O controle das faixas, que você disse que ainda está ruim. Três alturas, cores por estado e nada de botões sem rótulo.",
           mock, notas)

# ---------------------------------------------------------------------------------------------------------------
def tela_transporte():
    def barra(estado):
        play = "on" if estado in ("tocando", "gravando") else ""
        rec = "on" if estado == "gravando" else ""
        cor = {"parado": "var(--tx3)", "tocando": "var(--green)", "gravando": "var(--red)", "contagem": "var(--yellow)"}[estado]
        txt = {"parado": "PARADO", "tocando": "TOCANDO", "gravando": "GRAVANDO", "contagem": "CONTAGEM 2…"}[estado]
        return f'''<div style="display:flex;align-items:center;gap:8px;background:var(--bg2);border:1px solid var(--line);border-radius:10px;padding:10px 12px">
  <span class="btn ic">⏮</span><span class="btn ic">■</span><span class="btn ic {play}">▶</span><span class="btn ic rec {rec}" style="{'' if rec else 'color:var(--red)'}">●</span><span class="btn ic on">⟲</span>
  <div class="lcd" style="font-size:22px;border-color:{cor if estado!='parado' else '#000'}">003.2.040</div><div class="lcd" style="font-size:14px;color:var(--tx2)">0:05.214</div>
  <span style="font-size:11px;font-weight:800;color:{cor};letter-spacing:.08em;width:96px">{txt}</span>
  <span class="btn">♩ 140,00</span><span class="btn">4/4</span><span class="btn on ic">◷</span><span class="btn">Pré-roll 1</span></div>'''
    linhas = "".join(f'<div class="tag" style="font-size:12px;color:var(--tx2);margin:16px 0 6px">{e}</div>{barra(e.lower() if e!="Contagem antes de gravar" else "contagem")}' for e in ["Parado", "Tocando", "Gravando", "Contagem antes de gravar"])
    mock = f'''
<div style="display:flex;flex-direction:column;gap:6px;position:relative;width:1000px">
  <div class="tag" style="font-size:12px;color:var(--tx2)">Hoje</div>
  {recorte("atual-arranjo.png", 448, 24, 480, 60, 2)}
  {linhas}
  <div style="display:flex;gap:16px;margin-top:22px">
    <div style="background:var(--bg3);border:1px solid var(--line2);border-radius:8px;padding:10px;width:300px;position:relative"><div class="tag">Clique no tempo</div>
      <div style="margin-top:6px;font-size:12px;display:flex;flex-direction:column;gap:5px"><div>● Compassos e tempos</div><div>○ Minutos e segundos</div><div>○ Amostras</div><div>○ Timecode (SMPTE)</div></div>{b(5,280,-10)}</div>
    <div style="background:var(--bg3);border:1px solid var(--line2);border-radius:8px;padding:10px;width:300px;position:relative"><div class="tag">Clique no andamento</div>
      <div style="margin-top:6px;font-size:12px;display:flex;flex-direction:column;gap:5px"><div>Digitar o valor · arrastar ↕ · rodinha</div><div>Batidas para medir (Tap)</div><div>Abrir o mapa de tempo</div></div>{b(6,280,-10)}</div>
  </div>
  {b(1,-10,62)}{b(2,342,200)}{b(3,95,302)}{b(4,520,410)}
</div>'''
    notas = [
        "<b>Hoje o transporte mistura visores de LCD separados</b> (MIN, SEC, MSEG, TEMPO, COMPASSO) com rótulos minúsculos embaixo. Fica difícil de ler de longe.",
        "<b>Um visor grande</b> com a posição em compassos e outro menor com o tempo, os dois clicáveis para trocar o formato.",
        "<b>Botões com estado por cor</b>: tocar fica verde, gravar vermelho, o loop aceso quando ligado.",
        "<b>Uma palavra diz o estado</b> (parado, tocando, gravando, contagem) e a borda do visor muda de cor. Gravando, ninguém tem dúvida.",
        "<b>O formato de tempo</b> escolhido num clique, sem menu escondido.",
        "<b>Andamento editável no lugar</b>: digitar, arrastar, rodinha do mouse, Tap tempo e atalho para o mapa de tempo.",
    ]
    pagina("03-transporte", "3 · Barra de transporte",
           "Os controles de tocar, gravar e o tempo, que hoje ficam em visores separados e pequenos.", mock, notas)

# ---------------------------------------------------------------------------------------------------------------
def tela_piano_roll():
    teclas = ""
    notas = ""
    for i in range(36):
        preta = (i % 12) in (1, 3, 6, 8, 10)
        nome = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"][(23 - i) % 12]
        na_escala = nome in ("C", "D", "E", "F", "G", "A", "B")
        teclas += f'<div style="height:22px;border-bottom:1px solid #222;background:{"#2a2f35" if preta else "#d9dee4"};color:{"#888" if preta else "#333"};font-size:9px;padding-left:4px;display:flex;align-items:center">{nome + str(5 - i//12) if nome=="C" else ""}</div>'
    linhas = "".join(f'<div style="height:22px;border-bottom:1px solid #1a1e22;background:{"#232830" if (i%12) in (1,3,6,8,10) else "#262c33"}"></div>' for i in range(36))
    grade = "".join(f'<div style="position:absolute;top:0;bottom:0;left:{i*40}px;width:1px;background:{"#3a424b" if i%4==0 else "#2c3238"}"></div>' for i in range(26))
    posicoes = [(0, 3, 4), (4, 5, 2), (6, 7, 2), (8, 3, 4), (12, 8, 4), (16, 10, 6), (22, 7, 2), (24, 3, 8)]
    for x, linha, comp in posicoes:
        sel = x in (12, 16)
        notas += f'<div style="position:absolute;left:{x*40+1}px;top:{linha*22+2}px;width:{comp*40-2}px;height:18px;border-radius:4px;background:{"#7ef0a8" if sel else "var(--green)"};box-shadow:{"0 0 0 2px #fff" if sel else "none"};font-size:9px;color:#0b1a10;padding-left:4px;line-height:18px">{"C#4" if x==0 else ""}</div>'
    fantasmas = "".join(f'<div style="position:absolute;left:{x*40+1}px;top:{l*22+2}px;width:{c*40-2}px;height:18px;border-radius:4px;border:1px dashed rgba(155,107,232,.7)"></div>' for x, l, c in [(2, 12, 2), (10, 14, 4), (20, 12, 4)])
    vel = "".join(f'<div style="position:absolute;left:{x*40+2}px;bottom:0;width:6px;height:{30+((x*13)%40)}px;background:var(--green);border-radius:2px 2px 0 0"></div>' for x, _, _ in posicoes)
    mock = f'''
<div class="win">
  <div style="height:40px;background:var(--bg2);border-bottom:1px solid var(--line);display:flex;align-items:center;gap:6px;padding:0 10px">
    <b style="margin-right:8px">Piano roll · <span style="color:#f2a457">Baixo</span></b>
    <span class="btn sm on">✎ Desenhar</span><span class="btn sm">⬚ Selecionar</span><span class="btn sm">✂ Cortar</span><span class="btn sm">⌫ Apagar</span>
    <div class="sep"></div><span class="btn sm">Grade 1/16 ▾</span><span class="btn sm">Duração: última ▾</span>
    <div class="sep"></div><span class="btn sm on">Escala: Dó maior ▾</span><span class="btn sm">Acorde ▾</span><span class="btn sm">Fantasma: Pad ▾</span>
    <div style="flex:1"></div><span class="btn sm">Quantizar</span><span class="btn sm">Humanizar</span><span class="btn sm ic">⋯</span>
  </div>
  <div style="display:flex;flex:1;min-height:0">
    <div style="width:52px">{teclas}</div>
    <div style="flex:1;position:relative;overflow:hidden">
      <div style="position:absolute;inset:0">{linhas}</div>{grade}{fantasmas}{notas}
      <div style="position:absolute;top:0;bottom:0;left:520px;width:2px;background:#fff"></div>
      <div style="position:absolute;left:478px;top:174px;width:330px;height:62px;border:1px dashed #fff;border-radius:4px;background:rgba(255,255,255,.04)"></div>
    </div>
  </div>
  <div style="height:120px;border-top:1px solid var(--line);display:flex">
    <div style="width:52px;background:var(--bg2);display:flex;flex-direction:column;gap:4px;padding:6px 4px">
      <span class="btn sm on" style="font-size:9px">Vel</span><span class="btn sm" style="font-size:9px">Pan</span><span class="btn sm" style="font-size:9px">Pitch</span><span class="btn sm" style="font-size:9px">+ CC</span></div>
    <div style="flex:1;position:relative;background:var(--bg1)">{vel}</div>
  </div>
  {b(1,140,10)}{b(2,560,10)}{b(3,742,10)}{b(4,980,10)}{b(5,8,60)}{b(6,840,160)}{b(7,520,300)}{b(8,8,560)}
</div>'''
    notas_txt = [
        "<b>O nome da faixa e a cor dela</b> no título: dá para saber o que se edita sem procurar.",
        "<b>Ferramentas com texto</b> e uma barra só (hoje são duas fileiras de ícones e várias caixas sem rótulo, como 'Sem t…').",
        "<b>Escala e acorde à vista</b>: as notas fora da escala ficam mais escuras nas linhas do fundo.",
        "<b>Quantizar e humanizar</b> como botões diretos, e o resto num menu ⋯.",
        "<b>Teclado com os nomes das oitavas</b> (C3, C4) e as teclas pretas destacadas.",
        "<b>Notas fantasmas</b> de outra faixa tracejadas na cor dela, para compor em cima de outra parte.",
        "<b>Seleção com laço tracejado</b> e as notas selecionadas contornadas; o cursor de reprodução bem visível.",
        "<b>Lanes de baixo com abas</b> (velocidade, pan, pitch, CC) no lugar da única lane de volume de hoje.",
    ]
    pagina("04-piano-roll", "4 · Piano roll", "Edição de notas: menos ícones soltos, mais contexto à vista.", mock, notas_txt)

# ---------------------------------------------------------------------------------------------------------------
def tela_instrumento():
    abas = "".join(f'<span class="btn sm {"on" if i==0 else ""}">{t}</span>' for i, t in enumerate(["Instrumento", "Envelopes e filtro", "Acordes e arpejo", "Efeitos (2)", "MIDI", "Afinação"]))
    mock = f'''
<div style="display:flex;gap:26px;height:100%">
  <div style="width:520px;display:flex;flex-direction:column;gap:10px">
    <div class="tag" style="font-size:12px;color:var(--tx2)">Hoje: janela flutuante com abas só de ícones</div>
    {recorte("atual-instrumento.png", 226, 86, 290, 375, 1.6, 796)}
    {b(1,10,40)}
  </div>
  <div style="flex:1;display:flex;flex-direction:column;gap:10px;position:relative">
    <div class="tag" style="font-size:12px;color:var(--tx2)">Proposta: painel encaixado (embaixo ou no inspetor), abas com texto</div>
    <div class="win" style="flex:none;height:560px">
      <div style="height:46px;background:var(--bg2);border-bottom:1px solid var(--line);display:flex;align-items:center;gap:10px;padding:0 12px">
        <div style="width:6px;height:26px;background:#f0913a;border-radius:2px"></div><b style="font-size:15px">Baixo</b><span style="color:var(--tx3)">TripleOscillator</span>
        <span class="btn sm">Trocar ▾</span><div style="flex:1"></div>
        <span class="btn sm">◀ Preset: Fat Bass ▶</span><span class="btn sm">Salvar preset</span><span class="btn sm ic">⇱</span>
      </div>
      <div style="height:36px;background:var(--bg1);border-bottom:1px solid var(--line);display:flex;align-items:center;gap:6px;padding:0 10px">{abas}</div>
      <div style="flex:1;display:flex;align-items:center;justify-content:center;color:var(--tx3);background:repeating-linear-gradient(45deg,#1f2428 0 10px,#22272c 10px 20px)">painel do plugin (o desenho próprio de cada instrumento)</div>
      <div style="height:62px;background:var(--bg2);border-top:1px solid var(--line);display:flex;align-items:center;gap:14px;padding:0 14px">
        <div style="display:flex;flex-direction:column;align-items:center;font-size:10px;color:var(--tx3)"><div class="knob"></div>VOL</div>
        <div style="display:flex;flex-direction:column;align-items:center;font-size:10px;color:var(--tx3)"><div class="knob"></div>PAN</div>
        <div style="display:flex;flex-direction:column;align-items:center;font-size:10px;color:var(--tx3)"><div class="knob"></div>ALTURA</div>
        <span class="btn sm">Faixa de notas: C1–C7</span><span class="btn sm">Canal do mixer: 3 ▾</span><div style="flex:1"></div><span class="btn sm">Teclado ▾</span>
      </div>
    </div>
    {b(2,-12,40)}{b(3,230,96)}{b(4,360,36)}{b(5,40,560)}{b(6,520,560)}
  </div>
</div>'''
    notas_txt = [
        "<b>Hoje a janela do instrumento flutua sobre o arranjo</b> e as seis abas são ícones pequenos (seta, envelope, linhas, FX, teclado, diapasão) que só se entendem passando o mouse.",
        "<b>Painel encaixado</b> embaixo (como o piano roll) ou no inspetor da direita; o botão ⇱ ainda solta numa janela para quem prefere.",
        "<b>Abas com texto</b>: Instrumento, Envelopes e filtro, Acordes e arpejo, Efeitos (com contador), MIDI, Afinação.",
        "<b>Presets no topo</b>, com anterior e próximo e o nome do preset atual; trocar o instrumento sem apagar a faixa.",
        "<b>Rodapé fixo</b> com volume, pan, altura, a faixa de notas e o canal do mixer, sempre no mesmo lugar para todos os instrumentos.",
        "<b>O teclado de teste vira opcional</b> (botão Teclado), para sobrar espaço para o painel do plugin.",
    ]
    pagina("05-janela-do-instrumento", "5 · Janela do instrumento", "Onde se ajusta cada instrumento: de janela flutuante a painel encaixado.", mock, notas_txt)

# ---------------------------------------------------------------------------------------------------------------
def tela_navegador():
    arvore = '''
<div style="display:flex;flex-direction:column;gap:2px;font-size:12.5px">
 <div class="btn" style="justify-content:flex-start;color:var(--tx3);margin-bottom:6px">⌕ Buscar em tudo…</div>
 <div style="display:flex;gap:4px;margin-bottom:8px"><span class="btn sm on">Instrumentos</span><span class="btn sm">Efeitos</span><span class="btn sm">Amostras</span><span class="btn sm">Presets</span><span class="btn sm">Projetos</span></div>
 <div style="color:#4afd85;font-weight:700">▾ Sintetizadores</div>
 <div style="padding-left:14px;color:var(--tx2)">▾ Subtrativos</div>
 <div style="padding-left:30px">TripleOscillator <span style="color:var(--yellow)">★</span></div><div style="padding-left:30px">LB302</div><div style="padding-left:30px">Monstro</div>
 <div style="padding-left:14px;color:var(--tx2)">▸ Tabela de onda</div><div style="padding-left:14px;color:var(--tx2)">▸ Por expressão</div><div style="padding-left:14px;color:var(--tx2)">▸ Completos</div><div style="padding-left:14px;color:var(--tx2)">▸ Cordas</div>
 <div style="color:#f0913a;font-weight:700">▸ Samplers</div><div style="color:#e85a4f;font-weight:700">▸ Baterias</div><div style="color:#c8a06b;font-weight:700">▸ Modelagem física</div><div style="color:#9b6be8;font-weight:700">▸ Chiptune</div><div style="color:#9aa3ad;font-weight:700">▸ Hosts de plugins</div>
 <div style="color:#e85a4f;font-weight:700;margin-top:6px">▸ Guitarix (efeitos)</div>
</div>'''
    detalhe = '''
<div style="background:var(--bg2);border:1px solid var(--line);border-radius:10px;padding:14px;display:flex;flex-direction:column;gap:10px">
 <div style="display:flex;gap:10px;align-items:center"><div style="width:48px;height:48px;border-radius:8px;background:linear-gradient(135deg,#4afd85,#1f8f4c)"></div><div><b style="font-size:16px">TripleOscillator</b><div style="color:var(--tx3);font-size:12px">Sintetizador subtrativo · 3 osciladores</div></div><span style="margin-left:auto;color:var(--yellow);font-size:20px">★</span></div>
 <div style="color:var(--tx2);font-size:12.5px;line-height:1.5">Três osciladores com modulação entre eles (FM, AM, PM), ótimo para baixos, leads e pads.</div>
 <div class="tag">Presets (24)</div>
 <div style="display:grid;grid-template-columns:1fr 1fr;gap:5px;font-size:12px"><span class="btn sm on">▶ Fat Bass</span><span class="btn sm">▶ Soft Pad</span><span class="btn sm">▶ Pluck</span><span class="btn sm">▶ Lead Saw</span></div>
 <div style="display:flex;gap:6px;margin-top:4px"><span class="btn on">+ Nova faixa com ele</span><span class="btn">Trocar na faixa selecionada</span></div>
</div>'''
    mock = f'''
<div style="display:flex;gap:24px;height:100%">
  <div style="width:520px;display:flex;flex-direction:column;gap:10px">
    <div class="tag" style="font-size:12px;color:var(--tx2)">Hoje</div>
    {recorte("atual-arranjo.png", 0, 86, 214, 400, 1.5)}
  </div>
  <div style="flex:1;display:flex;gap:16px;position:relative">
    <div style="width:290px;background:var(--bg1);border:1px solid var(--line);border-radius:10px;padding:12px">{arvore}</div>
    <div style="flex:1">{detalhe}
      <div class="tag" style="margin-top:18px;font-size:12px;color:var(--tx2)">Amostras: pré-escuta</div>
      <div style="background:var(--bg2);border:1px solid var(--line);border-radius:10px;padding:10px;margin-top:8px;display:flex;flex-direction:column;gap:6px;font-size:12px">
        <div style="display:flex;gap:8px;align-items:center"><span class="btn sm ic on">▶</span>kick01.ogg <span style="color:var(--tx3);margin-left:auto">0,4 s · 44,1 kHz</span></div>
        <svg width="100%" height="36" viewBox="0 0 100 36" preserveAspectRatio="none"><path d="M0 18 L3 4 L6 32 L9 8 L12 28 L15 12 L20 24 L28 16 L40 20 L60 18 L100 18" stroke="#4afd85" fill="none" stroke-width="1"/></svg>
        <div style="display:flex;gap:8px;align-items:center"><span class="btn sm ic">▶</span>snare02.ogg <span style="color:var(--tx3);margin-left:auto">0,3 s</span></div>
        <div style="display:flex;gap:6px"><span class="btn sm on">Tocar ao selecionar</span><span class="btn sm">No andamento do projeto</span></div>
      </div>
    </div>
    {b(1,-10,-6)}{b(2,-10,64)}{b(3,-10,150)}{b(4,-10,340)}{b(5,300,-6)}{b(6,610,28)}{b(7,300,400)}
  </div>
</div>'''
    notas_txt = [
        "<b>Uma busca só</b> para instrumentos, efeitos, amostras, presets e projetos.",
        "<b>Abas por tipo</b> no topo do navegador, em vez de vários painéis separados.",
        "<b>Subcategorias</b> (como você pediu): Sintetizadores → Subtrativos, Tabela de onda, Por expressão, Completos, Cordas. O Vibed vai para Cordas.",
        "<b>Guitarix como categoria própria</b> nos efeitos, com as peças dentro.",
        "<b>Painel de detalhe</b> ao selecionar: o que o plugin faz, os presets dele com pré-escuta e os botões 'Nova faixa' e 'Trocar'.",
        "<b>Favoritos</b> (★) que sobem para o topo e uma lista de recentes.",
        "<b>Pré-escuta das amostras</b> com a forma de onda, duração e a opção de tocar no andamento do projeto.",
    ]
    pagina("06-navegador", "6 · Navegador de instrumentos, efeitos e amostras",
           "Achar e experimentar sons mais rápido: busca única, subcategorias e pré-escuta.", mock, notas_txt)

# ---------------------------------------------------------------------------------------------------------------
def tela_controles():
    def linha(titulo, conteudo):
        return f'<div style="display:flex;align-items:center;gap:14px;margin-bottom:16px"><div style="width:150px;color:var(--tx2);font-size:12px">{titulo}</div><div style="display:flex;gap:10px;align-items:center;flex-wrap:wrap">{conteudo}</div></div>'
    cores = "".join(f'<div style="display:flex;flex-direction:column;align-items:center;gap:4px;font-size:10px;color:var(--tx2)"><div style="width:46px;height:30px;border-radius:6px;background:{c}"></div>{n}</div>'
                    for c, n in [("var(--bg1)", "fundo"), ("var(--bg2)", "painel"), ("var(--bg3)", "botão"), ("var(--green)", "ação/ligado"), ("var(--red)", "gravar"), ("var(--orange)", "mudo"), ("var(--yellow)", "solo"), ("var(--blue)", "monitor/info")])
    mock = f'''
<div style="display:flex;flex-direction:column;position:relative;padding:6px 10px">
  <div class="tag" style="font-size:12px;color:var(--tx2);margin-bottom:14px">Um conjunto só de peças para o programa inteiro</div>
  {linha("Botão: estados", '<span class="btn">Normal</span><span class="btn" style="background:#3a424b">Passando o mouse</span><span class="btn on">Ligado</span><span class="btn" style="opacity:.4">Desligado</span><span class="btn" style="box-shadow:0 0 0 2px var(--blue)">Com foco (teclado)</span>')}
  {linha("Tamanhos", '<span class="btn sm">Pequeno 20 px</span><span class="btn">Normal 28 px</span><span class="btn" style="height:36px;font-size:14px;padding:0 16px">Grande 36 px</span><span style="color:var(--tx3);font-size:11px">área de clique mínima de 20 px</span>')}
  {linha("Com ícone e texto", '<span class="btn">✎ Desenhar</span><span class="btn">✂ Cortar</span><span class="btn ic">⋯</span><span style="color:var(--tx3);font-size:11px">só ícone quando o significado é universal (▶ ■ ● ⋯)</span>')}
  {linha("Estados de faixa", '<span class="btn sm ic rec on">●</span><span class="btn sm ic mon on">◉</span><span class="btn sm ic mute on">M</span><span class="btn sm ic solo on">S</span><span class="btn sm on">FX 2</span><span style="color:var(--tx3);font-size:11px">a mesma cor no cabeçalho, no mixer e no inspetor</span>')}
  {linha("Knob", '<div style="display:flex;flex-direction:column;align-items:center;gap:3px;font-size:10px;color:var(--tx3)"><div class="knob"></div>CUTOFF<span style="color:var(--tx)">1,2 kHz</span></div><div style="display:flex;flex-direction:column;align-items:center;gap:3px;font-size:10px;color:var(--tx3)"><div class="knob" style="box-shadow:0 0 0 2px var(--green)"></div>AUTOMATIZADO<span style="color:var(--green)">●</span></div><span style="color:var(--tx3);font-size:11px;max-width:420px">valor sempre visível embaixo; arrastar ↕, Shift para ajuste fino, duplo clique para digitar, Alt+clique volta ao padrão; anel verde quando automatizado ou ligado a um controlador</span>')}
  {linha("Régua (fader)", '<div style="width:220px"><div class="slider"><i style="width:65%"></i><b style="left:calc(65% - 3px)"></b></div></div><span style="font-size:11px">-4,2 dB</span><span style="color:var(--tx3);font-size:11px">volume e mixagem: régua em vez de knob</span>')}
  {linha("Lista e seleção", '<span class="btn" style="width:180px;justify-content:space-between">Entrada 1 (mono) <span>▾</span></span><span class="btn" style="width:140px;justify-content:space-between;color:var(--tx3)">⌕ Buscar…</span><span class="btn sm on">Sim</span><span class="btn sm">Não</span>')}
  {linha("Cores do tema", cores)}
  {linha("Texto", '<span style="font-size:18px;font-weight:700">Título 18</span><span style="font-size:14px;font-weight:600">Seção 14</span><span style="font-size:13px">Texto 13</span><span style="font-size:11px;color:var(--tx2)">Rótulo 11</span><span class="tag">ETIQUETA 10</span>')}
  {linha("Visores", '<span class="lcd" style="font-size:18px">003.2.040</span><span class="lcd" style="font-size:13px;color:var(--tx2)">0:05.214</span><span style="color:var(--tx3);font-size:11px">fonte de largura fixa, sem os dígitos de 7 segmentos borrados</span>')}
  {b(1,140,40)}{b(2,140,92)}{b(3,140,140)}{b(4,140,188)}{b(5,140,250)}{b(6,140,318)}{b(7,140,412)}{b(8,140,470)}
</div>'''
    notas = [
        "<b>Cinco estados claros</b> para todo botão: normal, mouse em cima, ligado (verde), desligado e com foco de teclado, para usar o programa sem mouse também.",
        "<b>Três tamanhos</b> e uma área de clique mínima de 20 px. Hoje há botões de 14 px.",
        "<b>Ícone com texto</b> nas ferramentas; só ícone quando o símbolo é universal.",
        "<b>Cores de estado iguais em todo lugar</b>: gravar vermelho, monitor azul, mudo laranja, solo amarelo, efeitos verde.",
        "<b>Knob com o valor sempre à vista</b> e gestos iguais em todos: arrastar, Shift para ajuste fino, duplo clique para digitar, Alt+clique para o padrão. O anel verde mostra automação.",
        "<b>Régua para volume</b>, mais precisa e legível que knob pequeno.",
        "<b>As mesmas cores do tema atual</b> (fundo cinza-azulado escuro e o verde do LMMS), só organizadas num jogo de nomes usado por todas as telas.",
        "<b>Visores com fonte legível</b> no lugar dos dígitos de 7 segmentos, que borram em telas pequenas.",
    ]
    pagina("07-botoes-e-controles", "7 · Botões, controles e cores",
           "A base que todas as outras telas usam: tamanhos, estados, cores e gestos iguais em todo o programa.", mock, notas)

# ---------------------------------------------------------------------------------------------------------------
def tela_configuracoes():
    secoes = ["Geral", "Áudio", "MIDI", "Gravação", "Plugins", "Pastas", "Aparência", "Teclado e atalhos", "Avançado"]
    lista = "".join(f'<div style="padding:9px 14px;border-radius:6px;{"background:var(--bg3);color:#fff;font-weight:600" if s=="Áudio" else "color:var(--tx2)"}">{s}</div>' for s in secoes)
    def campo(rotulo, valor, nota="", reinicio=False):
        tag = '<span style="font-size:10px;color:var(--yellow);border:1px solid var(--yellow);border-radius:4px;padding:1px 5px;margin-left:8px">pede reinício</span>' if reinicio else '<span style="font-size:10px;color:var(--green);margin-left:8px">aplica na hora</span>'
        return f'<div style="display:flex;align-items:center;gap:14px;padding:10px 0;border-bottom:1px solid #22282e"><div style="width:200px">{rotulo}{tag if nota!="-" else ""}</div><div>{valor}</div><div style="color:var(--tx3);font-size:11.5px;flex:1">{nota if nota!="-" else ""}</div></div>'
    mock = f'''
<div class="win" style="flex:none;height:760px;flex-direction:row">
  <div style="width:220px;background:var(--bg2);border-right:1px solid var(--line);padding:14px 10px;display:flex;flex-direction:column;gap:2px">
    <div class="btn" style="justify-content:flex-start;color:var(--tx3);margin-bottom:10px">⌕ Buscar configuração…</div>{lista}</div>
  <div style="flex:1;padding:20px 26px;display:flex;flex-direction:column;position:relative">
    <b style="font-size:18px">Áudio</b><div style="color:var(--tx3);font-size:12px;margin:4px 0 14px">Tudo aqui vale assim que muda: o dispositivo abre de novo sozinho.</div>
    {campo("Sistema de áudio", '<span class="btn" style="width:260px;justify-content:space-between">CoreAudio ▾</span>', "JACK, PortAudio, SDL… conforme o sistema")}
    {campo("Dispositivo de saída", '<span class="btn" style="width:260px;justify-content:space-between">Alto-falantes do MacBook ▾</span>')}
    {campo("Dispositivo de entrada", '<span class="btn" style="width:260px;justify-content:space-between">Microfone do MacBook ▾</span>')}
    {campo("Taxa de amostragem", '<span class="btn sm">44,1</span> <span class="btn sm on">48</span> <span class="btn sm">88,2</span> <span class="btn sm">96 kHz</span>')}
    {campo("Tamanho do buffer", '<div style="display:flex;align-items:center;gap:10px"><div class="slider" style="width:180px"><i style="width:30%"></i><b style="left:calc(30% - 3px)"></b></div><span>256 amostras</span></div>', "5,3 ms de latência de saída (medida do dispositivo)")}
    {campo("Idioma da interface", '<span class="btn" style="width:260px;justify-content:space-between">Português ▾</span>', "", True)}
    <div style="margin-top:auto;background:#2a2615;border:1px solid #6b5a1a;border-radius:8px;padding:12px 14px;display:flex;align-items:center;gap:12px">
      <span style="color:var(--yellow);font-size:18px">⟳</span><div style="flex:1;font-size:12.5px">O idioma muda depois de reiniciar o programa.</div><span class="btn">Depois</span><span class="btn on">Reiniciar agora</span></div>
    {b(1,100,72)}{b(2,128,316)}{b(3,600,290)}{b(4,-14,640)}
  </div>
  {b(5,10,8)}
</div>'''
    notas = [
        "<b>Aplica na hora</b>: trocar o sistema de áudio, o dispositivo, a taxa ou o buffer reabre o áudio sozinho, sem precisar fechar a tela nem reiniciar (você contou que hoje o backend não muda).",
        "<b>Cada opção diz se vale na hora</b> ou se pede reinício, antes de você mudar.",
        "<b>A latência real</b> medida do dispositivo, ao lado do buffer, em vez de só o número de amostras.",
        "<b>O que pede reinício</b> (idioma, tema) mostra uma faixa com 'Reiniciar agora', que salva o projeto e abre de novo.",
        "<b>Seções numa lista lateral com busca</b>, no lugar das abas de ícones de hoje.",
    ]
    pagina("08-configuracoes", "8 · Configurações", "Tudo o que dá para aplicar na hora é aplicado; o resto avisa e oferece reiniciar.", mock, notas)

# ---------------------------------------------------------------------------------------------------------------
def tela_sequencias():
    pecas = [("Bumbo", "#e85a4f", "x...x...x...x..."), ("Caixa", "#f0913a", "....x.......x..."), ("Chimbal fechado", "#f2d73a", "x.x.x.x.x.x.x.x."),
             ("Chimbal aberto", "#3dc8d8", ".......x.......x"), ("Palma", "#9b6be8", "....x.......x.x.")]
    linhas = ""
    for nome, cor, padrao in pecas:
        passos = "".join(f'<div style="width:34px;height:34px;border-radius:5px;background:{cor if c=="x" else ("#2c333b" if (i//4)%2==0 else "#262c33")};{"box-shadow:inset 0 -6px 0 rgba(0,0,0,.25)" if c=="x" else ""}"></div>' for i, c in enumerate(padrao))
        linhas += f'''<div style="display:flex;align-items:center;gap:6px;margin-bottom:6px">
<div style="width:5px;height:34px;background:{cor};border-radius:2px"></div><div style="width:150px;font-weight:600">{nome}</div>
<span class="btn sm ic mute">M</span><span class="btn sm ic solo">S</span><div class="slider" style="width:60px"><i style="width:70%"></i></div>
<div style="display:flex;gap:4px;margin-left:10px">{passos}</div></div>'''
    mock = f'''
<div class="win" style="flex:none;height:520px">
  <div style="height:44px;background:var(--bg2);border-bottom:1px solid var(--line);display:flex;align-items:center;gap:8px;padding:0 12px">
    <b>Sequência</b><span class="btn">◀ Batida A ▶</span><span class="btn sm">+ Nova</span><span class="btn sm">Duplicar</span><div class="sep"></div>
    <span class="btn sm">Passos: 16 ▾</span><span class="btn sm">Swing 0% ▾</span><div style="flex:1"></div><span class="btn sm on">▶ Tocar sequência</span>
  </div>
  <div style="padding:14px;position:relative">{linhas}
    <div style="position:absolute;left:{14+5+6+150+6+20+6+20+6+60+10+6+4*38}px;top:8px;width:36px;height:{len(pecas)*40}px;border:2px solid #fff;border-radius:6px;opacity:.6"></div>
    <div class="btn sm ghost" style="margin-top:6px">+ peça (arraste uma amostra ou instrumento)</div>
  </div>
  {b(1,200,10)}{b(2,500,10)}{b(3,170,100)}{b(4,440,60)}{b(5,30,300)}
</div>'''
    notas = [
        "<b>Troca de sequência à vista</b> (◀ Batida A ▶), com nova e duplicar ao lado.",
        "<b>Swing e número de passos</b> como botões diretos.",
        "<b>Cada peça com a cor e o nome legível</b>, mudo, solo e volume na mesma linha.",
        "<b>Passos grandes, agrupados de 4 em 4</b> pelo tom do fundo, e a coluna do passo atual destacada ao tocar.",
        "<b>Adicionar peça arrastando</b> uma amostra ou instrumento do navegador.",
    ]
    pagina("09-editor-de-sequencias", "9 · Editor de sequências (batidas)",
           "Para montar batidas mais rápido: passos maiores, grupos visíveis e o passo que toca destacado.", mock, notas)

if __name__ == "__main__":
    tela_principal(); tela_faixas(); tela_transporte(); tela_piano_roll(); tela_instrumento(); tela_navegador()
    tela_controles(); tela_configuracoes(); tela_sequencias()
