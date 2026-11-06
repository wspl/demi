<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>Seu agente, em todas as suas máquinas.<br>
  Um agente de IA na web que você hospeda no seu próprio servidor.</strong>
</p>

<p align="center">
  <a href="#licença"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.de.md">Deutsch</a> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <b>Português</b> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="O Demi em um navegador: um agente monta um layout de duas colunas para um app de iPhone, com um comentário e uma pergunta paralela na resposta e respostas sugeridas abaixo dela, o app em um iPhone Duo meio dobrado no painel lateral e a mesma conversa em um celular." width="800">
</p>

O Demi é um agente de IA web que você hospeda no seu próprio servidor. Abra
em qualquer navegador e entregue qualquer coisa a ele: pesquisa, sites,
arquivos ou código. Suas conversas ficam no seu servidor, e o trabalho roda
em um sandbox na nuvem ou nas suas próprias máquinas, mesmo depois de fechar
a aba.

> [!NOTE]
> O Demi está em fase inicial de desenvolvimento. Espere mudanças
> incompatíveis entre versões.

## Início rápido

Implante o Demi em poucos cliques:

<p>
  <a href="docs/delivery/railway.md"><img alt="Deploy on Railway" src="https://railway.com/button.svg" height="36"></a>
  <a href="https://render.com/deploy?repo=https://github.com/demi-sh/demi"><img alt="Deploy on Render" src="docs/assets/deploy/deploy-on-render.svg" height="36"></a>
  <a href="docs/delivery/fly.md"><img alt="Deploy on Fly.io" src="docs/assets/deploy/deploy-on-fly.svg" height="36"></a>
  <a href="docs/delivery/docker-compose.md"><img alt="Run with Docker Compose" src="docs/assets/deploy/run-with-docker-compose.svg" height="36"></a>
</p>

Ou instale em um servidor Linux próprio, com um nome de domínio:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

Ou peça a um agente de programação, como Claude Code ou Codex, que instale:

```text
Instale o Demi no meu servidor seguindo https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md
```

Depois abra o Demi, crie a conta de administrador e adicione um modelo em
**Configurações › Provedores**. O [guia de instalação](docs/delivery/installation.md)
cobre os sistemas compatíveis, HTTPS, o canal Canary e quanto custa cada
opção.

<p align="center">
  <img src="docs/assets/concept.webp" alt="De um computador ou de um celular, você acessa um servidor do Demi, que guarda todas as conversas e trabalha por meio de runners no seu notebook, desktop, HomeLab e VPS, ou em sandboxes na nuvem." width="800">
</p>

## Principais recursos

### Conversar, trabalhar e programar

- **Uma conversa para tudo**: perguntas rápidas e tarefas de várias etapas.
- **Programação**: edite repositórios, rode builds e testes, revise cada mudança como diff.
- **Pesquisa**: busque na web e leia páginas, PDFs e documentos.
- **Navegador de verdade**: o Demi controla o Chrome. Acompanhe e assuma quando quiser.
- **Apps de celular**: instale e teste apps em simuladores iOS e emuladores Android.
- **Arquivos e dados**: documentos, planilhas e imagens, devolvidos como anexos.
- **Imagens, vídeo e áudio**: feitos pelos provedores de modelos que você adicionar.
- **Comentários e perguntas**: selecione palavras de uma resposta para perguntar sobre elas.
- **Compartilhar e buscar**: compartilhe um link somente leitura e busque em cada mensagem.

### Em qualquer lugar, em qualquer máquina

- **Qualquer navegador**: computador, tablet, celular ou dobrável, sem instalar nada.
- **Sempre ativo**: as tarefas continuam depois de fechar a aba e avisam você.
- **Um só lugar para as conversas**: comece no notebook e continue no celular.
- **Sandbox privado na nuvem**: cada usuário tem seu próprio sandbox Linux.
- **Seus próprios computadores**: conecte macOS, Windows e Linux com um comando.
- **Várias máquinas, uma tarefa**: compile no Mac, teste no Windows, troque no meio da tarefa.
- **Arquivos em todo lugar**: navegue, visualize e busque os arquivos de cada máquina.
- **Encaminhamento de portas**: acesse uma porta de uma máquina a partir de outra.
- **Entre do seu jeito**: código QR ou passkey, em nove idiomas.

### Agente

- **Multiagente**: uma equipe de agentes divide uma tarefa grande e trabalha em paralelo.
- **Perfis e fluxos de trabalho**: agentes especialistas, compartilhados pelo seu repositório.
- **Agendamentos**: acorde uma conversa por timer, por cron ou quando algo mudar.
- **Conversas colaboram**: uma conversa envia mensagens a outra ou a lê.
- **Skills**: ensine novas tarefas ao Demi.
- **Credenciais seguras**: segredos passam por um formulário, nunca pela conversa.

### Modelos e equipes

- **Suas assinaturas**: Claude, ChatGPT e Grok Build, alternando contas nos limites.
- **Qualquer API**: Anthropic, OpenAI, Gemini, planos de programação como Kimi, GLM ou MiniMax, e modelos locais.
- **Troque de modelo quando quiser**, até no meio de uma tarefa.
- **Uso e custo**: veja os limites de cada assinatura e quanto o trabalho custou.
- **Multiusuário**: cada usuário tem seu próprio sandbox, dispositivos e conversas.

## Um agente construído sobre um shell

O Demi não é um wrapper do Claude Code nem do Codex: seu próprio runtime
controla cada modelo. Onde a maioria dos agentes dá ao modelo uma longa lista
de ferramentas, o Demi dá uma, um shell. O navegador, outras máquinas e os
subagentes são comandos `demi` dentro dele:

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **Combinável**: os comandos se encadeiam com pipes, então um passo faz o trabalho de várias chamadas de ferramenta.
- **Prompts enxutos**: o modelo lê o `--help` de um comando só quando precisa.
- **O mesmo shell em todo lugar**: um shell compatível com bash, escrito em Rust, em cada máquina.
- **Igual para todo modelo**: trocar de modelo não muda o que o agente consegue fazer.
- **Feito para tarefas longas**: os comandos rodam em segundo plano e avisam quando terminam.

> [!NOTE]
> **Planejado**: uma OpenAPI para o servidor do Demi, para que seus próprios
> programas usem as conversas, os agentes e os sandboxes na nuvem.

## Arquitetura

```text
  Browsers (computer, phone, tablet)
        │  HTTPS + WebSocket
        ▼
  ┌──────────────────────────┐   HTTPS   ┌────────────────────┐
  │ Demi server              │──────────▶│ Model providers    │
  │ users, conversations,    │           └────────────────────┘
  │ agents                   │
  └──────────────────────────┘
        ▲                ▲
        │ WebSocket      │ WebSocket
  ┌─────┴──────────┐  ┌──┴───────────────────────────┐
  │ Runner in your │  │ Runners on your computers    │
  │ cloud sandbox  │  │ and servers (macOS, Windows, │
  │                │  │ Linux)                       │
  └────────────────┘  └──────────────────────────────┘
```

- **O servidor roda o agente, os runners fazem o trabalho**: cada runner se
  conecta para fora, então funciona atrás de NAT e firewalls.
- **Um runner em todo lugar**: cada recurso funciona em cada máquina.
- **Leve na banda**: arquivos grandes ficam onde foram criados até você abri-los.
- **Nativo**: Rust no servidor, no runner e nas ferramentas; Vue e TypeScript no app web.

Leia a [documentação de design](docs/README.md), começando pela [visão
geral](docs/overview.md) (em inglês).

## Comparação

Como o Demi se diferencia do OpenHands, Orca, T3 Code, DeepSeek Harness,
opencode, Pi e herdr, recurso por recurso:
[Demi comparado](docs/comparison.pt-BR.md).

## Documentação

- [Documentação de design](docs/README.md): como cada parte do Demi funciona
- [Contribuir](CONTRIBUTING.md): ambiente de desenvolvimento, verificações e
  como adicionar um provedor
- [Segurança](SECURITY.md): como relatar vulnerabilidades e onde os dados são
  armazenados

Esses documentos estão em inglês.

## Licença

O Demi é publicado sob a [Apache License 2.0](LICENSE).
