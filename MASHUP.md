# Kingdom Come: Tarnished — como funciona o mashup

Este projeto transfere para **Elden Ring** o combate direcional de espada de
**Kingdom Come: Deliverance II (KCD2)**. Elden Ring continua sendo o jogo em
execução: movimentação, física, dano, equipamento, inimigos e animações são os
do jogo original. O mod intercepta parte dos controles e usa os dados e a
interface de KCD2 para reinterpretar ataques, defesa e feedback visual/sonoro.

## Visão geral da integração

O mod é uma DLL nativa escrita em Rust, carregada por ModEngine2. Não é uma
conversão de assets nem uma cópia dos sistemas completos de nenhum dos jogos.
Ele combina:

- dados de combate, ícones e sons lidos da instalação local de KCD2;
- estado do jogador, alvo, arma, câmera, stamina e animações lidos de Elden Ring;
- eventos de comportamento Havok de Elden Ring para iniciar os golpes;
- uma camada ImGui desenhada sobre a imagem de Elden Ring.

No arranque, [`mod/src/kcd2.rs`](./mod/src/kcd2.rs) procura a instalação de KCD2 pelo ambiente
`KCT_KCD2_DIR` ou pelo arquivo `kctarnished.env` ao lado da DLL. Lê as entradas
dos arquivos `.pak` como ZIP: setores, custos de ataques e tempos de defesa em
XML, ícones DDS e, se disponível, o banco de sons FMOD. Os arquivos de KCD2 não
são incluídos como recursos do mod; vêm da instalação do jogador. Se os dados
essenciais não puderem ser carregados, a funcionalidade de combate não é
ativada e o erro é registrado/mostrado no overlay.

As regras declarativas ficam em [`sheets/`](./sheets/). [`tools/codegen.py`](./tools/codegen.py) transforma
essas tabelas em [`mod/src/generated.rs`](./mod/src/generated.rs), depois de validar as fontes com
[`tools/preflight.py`](./tools/preflight.py). Para alterar os mapeamentos, a fonte de edição é a tabela,
não o arquivo Rust gerado.

## Fluxo durante o jogo

Há dois passos recorrentes no thread do jogo ([`mod/src/lib.rs`](./mod/src/lib.rs)):

1. **Antes de Elden Ring consumir os controles** (`pre_input`): lê os controles
   pela camada de input do próprio jogo, identifica arma/alvo e, quando a
   condição se aplica, captura ou altera inputs.
2. **Depois da física** (`update`): executa golpes enfileirados, acompanha o
   movimento das armas, identifica bloqueios e atualiza o estado do HUD.

A mecânica fica ativa enquanto o jogador tem um alvo travado e uma arma da mão
direita cuja classe consta em [`sheets/weapon_classes.json`](./sheets/weapon_classes.json). Fora dessa
condição, o fluxo normal de Elden Ring é preservado. As entradas são lidas
através da camada de atribuição de teclas do jogo, então os remapeamentos do
jogador continuam valendo.

## Ofensa: zonas de KCD2 escolhem animações de Elden Ring

### Cursor direcional

KCD2 define quatro setores ativos para o combate: `head` (de cima), `upper_left`,
`upper_right` e `lower` (de baixo). Suas fronteiras angulares são lidas do XML
de KCD2 em runtime, em vez de serem fixadas no código. Os setores
`lower_left` e `lower_right` estão inativos nos dados usados pelo projeto.

Com o alvo travado e uma arma coberta:

- mouse ou analógico direito movimentam um cursor direcional;
- o setor em que o cursor cai escolhe a direção do próximo corte;
- o cursor fica na última direção selecionada quando o analógico é solto;
- esses movimentos são suprimidos para que não girem a câmera ao mesmo tempo.

O overlay desenha a estrela/setores e o cursor usando o ícone extraído da
instalação de KCD2.

### Ataque

O botão de ataque normal é convertido em **corte**; o ataque forte é convertido
em **estocada**. O mod suprime esses inputs originais e os guarda por até
`0,35 s` caso Elden Ring ainda não aceite outro ataque. Quando a ação pode
começar, a direção selecionada, o tipo de arma e o uso com uma ou duas mãos
determinam a linha de [`sheets/swing_map.json`](./sheets/swing_map.json).

A linha contém um nome de evento de comportamento de Elden Ring. Por exemplo,
uma espada reta usada com uma mão tem eventos diferentes para corte da
direita, da esquerda, de cima e de baixo. [`mod/src/behavior.rs`](./mod/src/behavior.rs) localiza a
função de evento do jogo e dispara a animação correspondente no personagem do
jogador; os eventos, portanto, continuam sendo animações de Elden Ring.

A tabela de espada reta tem medições do laboratório de movimentos do mod para
as quatro direções, em uma e duas mãos, e para a estocada. As linhas de
greatsword, katana e light greatsword também estão cadastradas, mas seus campos
de medição estão vazios: são mapeamentos candidatos e não devem ser tratados
como validados em jogo. Classes não cadastradas continuam com o comportamento
normal de Elden Ring.

O custo de stamina original de Elden Ring permanece. Além dele, o mod aplica o
custo adicional de KCD2 conforme:

```text
(custo relativo do tipo de ataque - 1)
    * BaseAttackStaminaCost de KCD2
    * escala configurada (0,5)
```

Os valores relativos e a stamina base são lidos dos XMLs de KCD2; por exemplo,
o custo relativo da estocada é `1,0` e o do corte é `1,2` na versão registrada
em [`MODLOG.md`](./MODLOG.md).

## Defesa: direção do golpe recebido, bloqueio e riposta

O mod observa a animação do inimigo travado. Animações dentro da faixa
configurada como ataque iniciam o rastreamento do golpe. [`mod/src/tracker.rs`](./mod/src/tracker.rs)
amostra a posição de um osso da arma/mão e projeta seu movimento no plano da
câmera. A direção de onde a lâmina **vem** — oposta ao movimento mais rápido —
é então convertida para um setor de KCD2.

Na primeira vez que encontra uma animação, o mod mostra o setor quando o golpe
fica rápido o bastante e aprende o setor final. Salva o resultado por
identidade do comportamento do inimigo e ID de animação em
`learned_zones.json` (gerido por [`mod/src/learned.rs`](./mod/src/learned.rs));
nas próximas ocorrências, pode mostrar a direção já no início da animação.
Estocadas são separadas de cortes usando o deslocamento
para dentro/fora da câmera. Se não for possível rastrear um osso, a direção é
considerada desconhecida: o indicador cobre todos os lados e o bloqueio não
exige acertar um setor específico.

Enquanto o jogador segura Guard:

- **Setor correto:** o input de guarda chega a Elden Ring; o bloqueio e a
  absorção de stamina são feitos pelo sistema nativo da arma/jogo.
- **Setor errado conhecido:** o mod suprime Guard durante aquela animação do
  inimigo, fazendo com que o golpe não seja bloqueado pelo lado incorreto.
- **Bloqueio perfeito:** ao detectar uma queda de stamina sem perda de HP
  durante a guarda, compara o tempo desde o início da guarda com a janela
  calculada a partir dos dados de KCD2. Se a janela e o setor forem válidos,
  restaura a stamina anterior ao impacto, zera o valor de poise do alvo e abre
  uma janela de riposta.
- **Riposta:** um ataque normal feito durante essa janela dispara o evento de
  contra-ataque pesado de Elden Ring (com evento próprio para uma ou duas mãos)
  em vez do golpe direcional. O ataque continua sujeito à aceitação de ação e
  ao custo de stamina de Elden Ring; o mod também aplica o adicional de KCD2.

Os tempos de bloqueio perfeito e riposta são medianas calculadas das linhas de
`combat_action_perfect_block.xml`. Segundo [`MODLOG.md`](./MODLOG.md), a versão de KCD2
analisada não tinha linhas de jogador nessa tabela; por isso os tempos atuais
derivam de 206 linhas de NPC, não de dados de animação do jogador.

## Interface e áudio

O overlay usa ImGui/hudhook na apresentação D3D12 de Elden Ring e mostra:

- estrela direcional e cursor;
- direção de ataque recebida perto da posição projetada do alvo;
- ícone temporário de bloqueio perfeito e indicador da janela de riposta;
- mensagens de erro ou informação de laboratório, quando aplicáveis.

Os ícones DDS vêm de KCD2. Os sons de choque também vêm do banco FMOD de KCD2:
o mod decodifica os samples Vorbis selecionados e toca variações para bloqueio
normal, bloqueio perfeito e início da riposta. O áudio é opcional; ícones e
tabelas de combate são fontes obrigatórias na configuração registrada.

## O que é nativo, o que é adaptado e o que falta validar

| Elemento | Origem/implementação |
|---|---|
| Movimento, travamento de alvo, dano e stamina base | Elden Ring |
| Controles direcionais, setores, janela temporal de bloqueio perfeito e feedback visual/sonoro | Regras e dados de KCD2 adaptados pelo mod |
| Animações de ataque e contra-ataque | Eventos de comportamento já existentes em Elden Ring |
| Bloqueio de setor correto | Guarda normal de Elden Ring |
| Setor de ataques inimigos | Medido a partir da animação/osso e da câmera de Elden Ring, convertido para os setores de KCD2 |
| Espada reta | Mapeamento de direção medido no laboratório, conforme `MODLOG.md` |
| Outras classes de arma na tabela | Candidatas ainda sem medições registradas |

O projeto documenta como próximos passos um teste de combate completo contra
inimigos reais, nova validação de linhas de uma mão e medições para as outras
classes. Assim, o indicador de ataque recebido e os mapeamentos não devem ser
interpretados como universalmente calibrados para todos os inimigos, armas ou
animações.

## Arquivos principais

- [`mod/src/lib.rs`](./mod/src/lib.rs): inicialização da DLL e agendamento dos dois passos de jogo.
- [`mod/src/combat.rs`](./mod/src/combat.rs): captura de inputs, seleção de ataques, regras de bloqueio e riposta.
- [`mod/src/kcd2.rs`](./mod/src/kcd2.rs): leitura dos dados e recursos da instalação de KCD2.
- [`mod/src/behavior.rs`](./mod/src/behavior.rs): invocação de eventos de animação de Elden Ring.
- [`mod/src/tracker.rs`](./mod/src/tracker.rs) e [`mod/src/learned.rs`](./mod/src/learned.rs): rastreamento e memorização de direções de golpes inimigos.
- [`mod/src/hud.rs`](./mod/src/hud.rs) e [`mod/src/audio.rs`](./mod/src/audio.rs): overlay e sons.
- [`sheets/`](./sheets/): configuração declarativa dos mapeamentos usados pelo gerador.
- [`MODLOG.md`](./MODLOG.md): notas de execução, fatos medidos e itens pendentes do projeto.
