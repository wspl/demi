# Demi comparado

[English](comparison.md) · [Deutsch](comparison.de.md) · [Español](comparison.es.md) · [Français](comparison.fr.md) · **Português** · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

Como o Demi se diferencia dos apps de agentes do mesmo tipo, em outubro de 2026, com base na documentação e no código-fonte de cada produto. A tabela mostra uma linha só onde os produtos diferem, além de um recurso que nenhum deles tem ainda. As colunas vão do produto mais próximo do Demi ao mais distante.

A tabela usa estes símbolos e rótulos:

- ✓ Compatível
- ◐ Em parte
- Em branco: não compatível
- **Web**: usado no navegador, sem app próprio
- **PWA**: instalado pelo navegador como um app

A tabela a seguir compara os produtos recurso por recurso:

| | Demi | [OpenHands][openhands] | [Orca][orca] | [T3 Code][t3-code] | [DeepSeek Harness][deepseek-harness] | [opencode][opencode] | [Pi][pi] | [herdr][herdr] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Modelos e assinaturas** | | | | | | | | |
| Assinatura do Claude | ✓ | ✓ | ✓ | ✓ | ◐¹ | | ✓ | ✓ |
| ChatGPT, Grok e outros | ✓ | ✓ | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ |
| Várias contas | ✓ | | ✓ | ✓ | | ✓ | | |
| Rodízio de contas | ✓ | | | | | | | |
| Uso da assinatura | ✓ | ◐² | ✓ | ✓ | ◐³ | | | |
| Troca de modelo no meio da tarefa | ✓ | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| **Interface** | | | | | | | | |
| App web | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Celular | PWA | Web | ✓ | ✓ | Web | Web | | |
| App para desktop | PWA | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Interface de terminal | | ✓ | | | | ✓ | ✓ | ✓ |
| Entrada por voz | | ✓ | ✓ | ✓ | ◐⁴ | | | |
| Notificações no celular | ✓ | | ✓ | ✓⁵ | | | | ◐⁶ |
| Busca nas conversas | ✓ | | ◐⁷ | ✓ | ◐⁸ | ◐⁸ | ✓ | |
| **Execução** | | | | | | | | |
| Servidor central | ✓ | ◐ | ◐ | | ◐ | ◐ | | |
| Funciona com todos os dispositivos desligados | ✓ | ✓⁹ | ◐ | ◐ | ◐ | ◐ | | ◐ |
| Conversas sincronizadas | ✓ | ◐ | ◐ | ◐ | | ◐ | | |
| Sandbox na nuvem | ✓ | ✓ | ◐ | | | | | |
| Máquinas remotas | ✓ | ◐ | ✓ | ✓ | ◐ | ◐ | ◐ | ✓ |
| Tarefas em várias máquinas | ✓ | | | | | | | |
| Troca de máquina | ✓ | | | | | | ◐ | |
| Encaminhamento de portas | ✓ | | ✓¹⁰ | | | | | |
| Tarefas agendadas | ✓ | ✓ | ✓ | ✓ | ✓ | ◐¹¹ | | |
| **Arquivos** | | | | | | | | |
| Navegador de arquivos | ✓ | ✓ | ✓ | ✓ | ◐¹² | ✓ | | |
| Gerenciamento de arquivos | ◐¹³ | ◐¹⁴ | ✓ | ◐¹⁵ | | | | |
| Busca no conteúdo dos arquivos | ✓ | | ✓ | ✓ | | | | |
| Worktrees do Git | ◐¹⁶ | ✓ | ✓ | ✓ | ◐⁴ | ✓ | | ✓ |
| Comentários por linha em diffs | | | ✓ | ◐¹⁷ | | ✓ | | |
| **Agente** | | | | | | | | |
| Agente próprio | ✓ | ✓ | | | ✓ | ✓ | ✓ | |
| Só um shell como ferramenta | ✓ | | | | | | | |
| Vários agentes | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| Perfis de agente | ✓ | ✓ | | | ✓ | ✓ | ◐ | |
| Habilidades | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| MCP | | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| Plugins de terceiros | | ✓ | ◐⁴ | | ✓ | ✓ | ✓ | ✓ |
| Credenciais para o agente | ✓ | ✓ | | | | | | |
| Automação do navegador | ✓ | ✓ | ✓ | ✓ | ◐ | ◐¹⁸ | | |
| Ver e controlar o navegador ao vivo | ✓ | ◐¹⁹ | ✓ | ✓ | | ◐¹⁸ | | |
| Uso do computador | | | ◐¹⁸ | ◐²⁰ | ◐⁴ | | | |
| **Dispositivos móveis** | | | | | | | | |
| Simulador do iOS | ✓ | | ◐¹⁸ | ✓ | | | | |
| Dispositivo iOS | | | | | | | | |
| Emulador do Android | ✓ | | ◐¹⁸ | ✓ | | | | |
| Dispositivo Android | | | ◐¹⁸ | ✓ | | | | |
| **Geração de mídia** | | | | | | | | |
| Geração e edição de imagens | ✓ | ◐²¹ | ◐²¹ | ◐²¹ | ◐²¹ | | ◐²² | ◐²¹ |
| Geração de vídeo | ✓ | | | | | | | |
| Fala, música e efeitos sonoros | ✓ | | | | | | | |
| **Equipes** | | | | | | | | |
| Vários usuários | ✓ | ◐⁹ | | | | | | |
| Console de administração | ✓ | ◐⁹ | | | | ◐⁹ | | |
| Isolamento por usuário | ✓ | ◐⁹ | | | | | | |
| Links de compartilhamento de conversas | ✓ | ◐²³ | | ◐²⁴ | ◐⁴ | | ✓ | |

As notas numeradas da tabela dizem o seguinte:

1. Só pelos subagentes Claude Code e Codex que ele inicia. O próprio login com assinatura está no código, mas não tem entrada no app.
2. Uma estimativa do uso do plano a preços de API.
3. Só a cota da própria conta da DeepSeek.
4. Experimental, ativado manualmente.
5. Só pelo T3 Connect.
6. Só por um plugin de exemplo que envia para o Telegram.
7. Por título, pasta, branch, modelo e prévia, não no texto inteiro.
8. Só por título.
9. Só na edição paga na nuvem ou empresarial.
10. Só de um host SSH para este computador; ele encontra para você as portas em escuta do host.
11. Só como um job do GitHub Actions em um agendamento cron.
12. Só os arquivos da máquina em que ele roda; as visualizações web dele não alcançam um host SSH.
13. Envio, download e novas pastas; sem renomear, excluir, mover ou editar.
14. Só o download do espaço de trabalho inteiro.
15. Só a edição do conteúdo de um arquivo.
16. Mostra os worktrees existentes sob o projeto, mas não cria nenhum.
17. Um comentário sobre um trecho citado de qualquer texto, não sobre uma linha de um diff.
18. Só no computador que executa o app para desktop dele, não no cliente web nem em uma máquina remota.
19. Mostra a última captura de tela do agente, que não pode ser operada.
20. Só simuladores de iOS e Android, não o desktop.
21. Pela ferramenta de imagens do agente Codex que ele executa.
22. Só a partir de um script do codemode, ativado manualmente.
23. Só com um backend do OpenHands Cloud.
24. Links de convidado, um beta que exige o T3 Connect.

[openhands]: https://github.com/OpenHands/OpenHands
[orca]: https://github.com/stablyai/orca
[t3-code]: https://github.com/pingdotgg/t3code
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[opencode]: https://github.com/anomalyco/opencode
[pi]: https://github.com/earendil-works/pi
[herdr]: https://github.com/herdrdev/herdr

## O que as linhas significam

As linhas da tabela significam o seguinte:

- **Linhas sobre uma máquina** (arquivos, worktrees, diffs, o navegador, o uso do computador e os dispositivos móveis): um recurso só é ✓ quando também funciona enquanto o agente trabalha em uma máquina remota, como um host SSH, outro computador ou uma máquina na nuvem. Um recurso que só funciona no computador que executa o produto é ◐.
- **Assinaturas**: entrar com uma assinatura em vez de uma chave de API. Conta uma assinatura usada por meio de um agente que o produto executa, como o Claude Code ou o Codex.
- **Várias contas**, **Rodízio de contas**: feitos pelo próprio produto, não por um agente que ele executa. O rodízio passa o trabalho para outra conta quando uma atinge o limite.
- **Uso da assinatura**: quanto dos limites de cada assinatura foi usado e quando cada limite é redefinido.
- **Notificações no celular**: uma notificação chega ao celular com o app fechado.
- **Busca nas conversas**: encontra qualquer palavra de qualquer mensagem em todas as conversas.
- **Servidor central**: um servidor guarda o agente e todas as conversas, e seus computadores só executam os comandos dele.
  - ◐: um servidor que executa comandos só na própria máquina.
  - Em branco: cada máquina executa um agente próprio, ou não há servidor.
- **Funciona com todos os dispositivos desligados**: uma tarefa continua enquanto seus computadores e seu celular estão desligados.
  - ◐: só quando você o instala em uma máquina sua que fica ligada.
- **Conversas sincronizadas**: todas as conversas ficam em uma só lista e aparecem iguais em cada dispositivo, seja qual for a máquina em que rodam.
  - ◐: sincronizadas só entre os dispositivos conectados a um mesmo servidor.
- **Tarefas em várias máquinas**: uma conversa trabalha em várias máquinas.
- **Troca de máquina**: uma conversa passa para outra máquina e continua.
- **Encaminhamento de portas**: um programa em uma máquina alcança uma porta de outra, como o `ssh -L` faz.
- **Tarefas agendadas**: uma tarefa começa sozinha em um horário ou intervalo definido.
- **Gerenciamento de arquivos**: enviar, baixar, criar, renomear, excluir, mover e editar arquivos no app.
- **Comentários por linha em diffs**: um comentário sobre uma linha de uma alteração, que vai para o agente.
- **Agente próprio**: o agente do próprio produto controla o modelo, em vez de executar o Claude Code, o Codex ou outros agentes.
- **Só um shell como ferramenta**: o modelo recebe uma única ferramenta, um shell, e cada recurso é um comando dentro dele.
- **Plugins de terceiros**: código de outra pessoa que o usuário instala para adicionar ferramentas ou telas ao produto, além das habilidades e dos servidores MCP. Os próprios recursos do Demi são plugins, mas ele não carrega os de mais ninguém.
- **Credenciais para o agente**: segredos que o usuário salva com um nome e que o agente usa nos comandos.
- **Ver e controlar o navegador ao vivo**: o usuário acompanha o navegador do agente enquanto ele trabalha e pode operá-lo.
- **Uso do computador**: o agente opera o desktop com capturas de tela, mouse e teclado, além de um navegador.
- **Dispositivo iOS**, **Dispositivo Android**: um celular de verdade conectado ao computador, não um simulador.
- **Links de compartilhamento de conversas**: um link que mostra uma conversa para outra pessoa.

<details>
<summary>Fontes</summary>

- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools), [Automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations), [Secrets](https://github.com/OpenHands/OpenHands/blob/main/src/routes/secrets-settings.tsx), [Files](https://github.com/OpenHands/OpenHands/blob/main/src/routes/files-tab.tsx), [Workspace download](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation-panel/conversation-card/conversation-card-context-menu.tsx), [Browser panel](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/browser/browser.tsx), [Public share](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation/conversation-name-context-menu.tsx), [Dictation](https://github.com/OpenHands/OpenHands/blob/main/src/hooks/chat/use-dictation.ts), [Worktrees](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/chat/workspace-mode-selector.tsx), [Usage](https://github.com/OpenHands/OpenHands/blob/main/src/routes/usage-tab.tsx), [Plugins](https://github.com/OpenHands/OpenHands/blob/main/src/routes/skills-plugins.tsx)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview), [File explorer](https://www.onorca.dev/docs/editing/file-explorer), [Editor](https://www.onorca.dev/docs/editing/monaco), [Session history](https://www.onorca.dev/docs/agents/session-history), [Worktrees](https://www.onorca.dev/docs/model/worktrees), [Annotate AI diffs](https://www.onorca.dev/docs/review/annotate-ai-diff), [SSH and port forwarding](https://www.onorca.dev/docs/ssh), [Automations](https://www.onorca.dev/docs/cli/automations), [Usage tracking](https://www.onorca.dev/docs/agents/usage-tracking), [Settings and plugins](https://www.onorca.dev/docs/settings), [iOS emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator/SKILL.md), [Android emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator-android/SKILL.md), [Client creation policy](https://github.com/stablyai/orca/blob/main/src/renderer/src/lib/client-creation-action-policy.ts)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md), [Usage and limits](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md), [Composer](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md), [Mobile notifications](https://github.com/pingdotgg/t3code/blob/main/docs/user/mobile-notifications.md), [Scheduled tasks](https://github.com/pingdotgg/t3code/blob/main/docs/user/project-settings.md), [Guest links](https://github.com/pingdotgg/t3code/blob/main/docs/user/guest-links.md), [Thread sidebar](https://github.com/pingdotgg/t3code/blob/main/docs/user/thread-sidebar.md), [Thread search](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ThreadSearch.ts), [File browser](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/FileBrowserPanel.tsx), [File saving](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/fileSaveCoordinator.ts), [Content search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/search/ProjectContentSearchDialog.tsx)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md), [Schedule](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/schedule.md), [Files sidebar](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-sidebar-files/README.md), [Workspace](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-workspace/README.md), [Plugin indexes](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/plugin-market.md), [Account quota](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-settings-account/src/client/AccountQuotaNotice.tsx), [Voice input](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/voice-input-bundle/README.md), [Liveshare](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/liveshare-protocol/README.md), [Worktree](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/worktree/README.md)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/), [GitHub](https://opencode.ai/docs/github/), [Plugins](https://opencode.ai/docs/plugins/), [Files](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/files/model.tsx), [Workspaces](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/submenu.tsx), [Session search](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/home/sessions/search.ts), [Review comments](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/composer/comment-note.ts), [AI library](https://github.com/anomalyco/opencode/blob/dev/packages/ai/README.md)
- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md), [Extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md), [Codemode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/codemode.md), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/), [Configuration](https://herdr.dev/docs/configuration/), [Plugins](https://herdr.dev/docs/plugins/)
- Codex: [Image generation skill](https://github.com/openai/codex/blob/main/codex-rs/skills/src/assets/samples/imagegen/SKILL.md)

</details>
