# Demi comparado

[English](comparison.md) · [Deutsch](comparison.de.md) · **Español (Latinoamérica)** · [Français](comparison.fr.md) · [Português (Brasil)](comparison.pt-BR.md) · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

En qué se diferencia Demi de las apps de agentes de su tipo, a octubre de 2026, según la documentación y el código fuente de cada producto. La tabla incluye una fila solo donde los productos difieren, además de una capacidad que ninguno tiene todavía. Sus columnas van del producto más cercano a Demi al más lejano.

La tabla usa estos símbolos y etiquetas:

- ✓ Compatible
- ◐ En parte
- En blanco: no compatible
- **Web**: se usa en el navegador, sin app propia
- **PWA**: se instala desde el navegador como app

La siguiente tabla compara los productos función por función:

| | Demi | [OpenHands][openhands] | [Orca][orca] | [T3 Code][t3-code] | [DeepSeek Harness][deepseek-harness] | [opencode][opencode] | [Pi][pi] | [herdr][herdr] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Modelos y suscripciones** | | | | | | | | |
| Suscripción de Claude | ✓ | ✓ | ✓ | ✓ | ◐¹ | | ✓ | ✓ |
| ChatGPT, Grok y otros | ✓ | ✓ | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ |
| Varias cuentas | ✓ | | ✓ | ✓ | | ✓ | | |
| Rotación de cuentas | ✓ | | | | | | | |
| Uso de la suscripción | ✓ | ◐² | ✓ | ✓ | ◐³ | | | |
| Cambio de modelo a mitad de tarea | ✓ | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| **Interfaz** | | | | | | | | |
| App web | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Móvil | PWA | Web | ✓ | ✓ | Web | Web | | |
| App de escritorio | PWA | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Interfaz de terminal | | ✓ | | | | ✓ | ✓ | ✓ |
| Entrada de voz | | ✓ | ✓ | ✓ | ◐⁴ | | | |
| Notificaciones en el teléfono | ✓ | | ✓ | ✓⁵ | | | | ◐⁶ |
| Búsqueda en chats | ✓ | | ◐⁷ | ✓ | ◐⁸ | ◐⁸ | ✓ | |
| **Ejecución** | | | | | | | | |
| Servidor central | ✓ | ◐ | ◐ | | ◐ | ◐ | | |
| Funciona con todos los dispositivos apagados | ✓ | ✓⁹ | ◐ | ◐ | ◐ | ◐ | | ◐ |
| Chats sincronizados | ✓ | ◐ | ◐ | ◐ | | ◐ | | |
| Sandbox en la nube | ✓ | ✓ | ◐ | | | | | |
| Máquinas remotas | ✓ | ◐ | ✓ | ✓ | ◐ | ◐ | ◐ | ✓ |
| Tareas en varias máquinas | ✓ | | | | | | | |
| Cambio de máquina | ✓ | | | | | | ◐ | |
| Reenvío de puertos | ✓ | | ✓¹⁰ | | | | | |
| Tareas programadas | ✓ | ✓ | ✓ | ✓ | ✓ | ◐¹¹ | | |
| **Archivos** | | | | | | | | |
| Explorador de archivos | ✓ | ✓ | ✓ | ✓ | ◐¹² | ✓ | | |
| Administración de archivos | ◐¹³ | ◐¹⁴ | ✓ | ◐¹⁵ | | | | |
| Búsqueda en el contenido de archivos | ✓ | | ✓ | ✓ | | | | |
| Worktrees de Git | ◐¹⁶ | ✓ | ✓ | ✓ | ◐⁴ | ✓ | | ✓ |
| Comentarios por línea en diffs | | | ✓ | ◐¹⁷ | | ✓ | | |
| **Agente** | | | | | | | | |
| Agente propio | ✓ | ✓ | | | ✓ | ✓ | ✓ | |
| Solo un shell como herramienta | ✓ | | | | | | | |
| Varios agentes | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| Perfiles de agente | ✓ | ✓ | | | ✓ | ✓ | ◐ | |
| Habilidades | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| MCP | | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| Plugins de terceros | | ✓ | ◐⁴ | | ✓ | ✓ | ✓ | ✓ |
| Credenciales para el agente | ✓ | ✓ | | | | | | |
| Automatización del navegador | ✓ | ✓ | ✓ | ✓ | ◐ | ◐¹⁸ | | |
| Ver y controlar el navegador en vivo | ✓ | ◐¹⁹ | ✓ | ✓ | | ◐¹⁸ | | |
| Uso de la computadora | | | ◐¹⁸ | ◐²⁰ | ◐⁴ | | | |
| **Dispositivos móviles** | | | | | | | | |
| Simulador de iOS | ✓ | | ◐¹⁸ | ✓ | | | | |
| Dispositivo iOS | | | | | | | | |
| Emulador de Android | ✓ | | ◐¹⁸ | ✓ | | | | |
| Dispositivo Android | | | ◐¹⁸ | ✓ | | | | |
| **Generación de medios** | | | | | | | | |
| Generación y edición de imágenes | ✓ | ◐²¹ | ◐²¹ | ◐²¹ | ◐²¹ | | ◐²² | ◐²¹ |
| Generación de video | ✓ | | | | | | | |
| Voz, música y efectos de sonido | ✓ | | | | | | | |
| **Equipos** | | | | | | | | |
| Varios usuarios | ✓ | ◐⁹ | | | | | | |
| Consola de administración | ✓ | ◐⁹ | | | | ◐⁹ | | |
| Aislamiento por usuario | ✓ | ◐⁹ | | | | | | |
| Enlaces para compartir chats | ✓ | ◐²³ | | ◐²⁴ | ◐⁴ | | ✓ | |

Las notas numeradas de la tabla dicen lo siguiente:

1. Solo a través de los subagentes Claude Code y Codex que inicia. Su propio inicio de sesión con suscripción está en el código, pero no tiene entrada en la app.
2. Una estimación del uso del plan a precios de API.
3. Solo la cuota de su propia cuenta de DeepSeek.
4. Experimental, se activa a mano.
5. Solo a través de T3 Connect.
6. Solo mediante un plugin de ejemplo que envía a Telegram.
7. Por título, carpeta, rama, modelo y vista previa, no en todo el texto.
8. Solo por título.
9. Solo en la edición de nube de pago o empresarial.
10. Solo de un host SSH a esta computadora; encuentra por ti los puertos en escucha del host.
11. Solo como un trabajo de GitHub Actions con una programación cron.
12. Solo los archivos de la máquina donde se ejecuta; sus vistas web no llegan a un host SSH.
13. Subir, descargar y crear carpetas; no se puede renombrar, eliminar, mover ni editar.
14. Solo descargar todo el espacio de trabajo.
15. Solo editar el contenido de un archivo.
16. Muestra los worktrees existentes bajo su proyecto, pero no crea ninguno.
17. Un comentario sobre un fragmento citado de cualquier texto, no sobre una línea de un diff.
18. Solo en la computadora que ejecuta su app de escritorio, no en su cliente web ni en una máquina remota.
19. Muestra la última captura de pantalla del agente, que no se puede operar.
20. Solo simuladores de iOS y Android, no el escritorio.
21. A través de la herramienta de imágenes del agente Codex que ejecuta.
22. Solo desde un script de codemode, que se activa a mano.
23. Solo con un backend de OpenHands Cloud.
24. Enlaces de invitado, una beta que requiere T3 Connect.

[openhands]: https://github.com/OpenHands/OpenHands
[orca]: https://github.com/stablyai/orca
[t3-code]: https://github.com/pingdotgg/t3code
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[opencode]: https://github.com/anomalyco/opencode
[pi]: https://github.com/earendil-works/pi
[herdr]: https://github.com/herdrdev/herdr

## Qué significan las filas

Las filas de la tabla significan lo siguiente:

- **Filas sobre una máquina** (archivos, worktrees, diffs, el navegador, el uso de la computadora y los dispositivos móviles): una función es ✓ solo cuando también funciona mientras el agente trabaja en una máquina remota, como un host SSH, otra computadora o una máquina en la nube. Una que funciona solo en la computadora que ejecuta el producto es ◐.
- **Suscripciones**: iniciar sesión con una suscripción en lugar de una clave de API. Cuenta una suscripción usada a través de un agente que ejecuta el producto, como Claude Code o Codex.
- **Varias cuentas**, **Rotación de cuentas**: las hace el propio producto, no un agente que ejecuta. La rotación pasa el trabajo a otra cuenta cuando una alcanza su límite.
- **Uso de la suscripción**: cuánto de los límites de cada suscripción se ha usado y cuándo se restablece cada límite.
- **Notificaciones en el teléfono**: una notificación llega al teléfono con la app cerrada.
- **Búsqueda en chats**: encuentra cualquier palabra de cualquier mensaje en todos los chats.
- **Servidor central**: un servidor contiene el agente y todos los chats, y tus computadoras solo ejecutan sus comandos.
  - ◐: un servidor que ejecuta comandos solo en su propia máquina.
  - En blanco: cada máquina ejecuta un agente propio, o no hay servidor.
- **Funciona con todos los dispositivos apagados**: una tarea sigue en marcha mientras tus computadoras y tu teléfono están apagados.
  - ◐: solo si lo instalas en una máquina propia que se queda encendida.
- **Chats sincronizados**: todos los chats están en una sola lista y se ven igual en cada dispositivo, sin importar en qué máquina se ejecuten.
  - ◐: sincronizados solo entre los dispositivos conectados a un mismo servidor.
- **Tareas en varias máquinas**: un chat trabaja en varias máquinas.
- **Cambio de máquina**: un chat pasa a otra máquina y continúa.
- **Reenvío de puertos**: un programa en una máquina llega a un puerto de otra, como lo hace `ssh -L`.
- **Tareas programadas**: una tarea empieza sola a una hora fija o cada cierto intervalo.
- **Administración de archivos**: subir, descargar, crear, renombrar, eliminar, mover y editar archivos en la app.
- **Comentarios por línea en diffs**: un comentario sobre una línea de un cambio, que va al agente.
- **Agente propio**: el agente del propio producto maneja el modelo, en lugar de ejecutar Claude Code, Codex u otros agentes.
- **Solo un shell como herramienta**: el modelo recibe una sola herramienta, un shell, y cada capacidad es un comando dentro de él.
- **Plugins de terceros**: código de otra persona que el usuario instala para agregar herramientas o pantallas al producto, más allá de las habilidades y los servidores MCP. Las funciones propias de Demi son plugins, pero no carga los de nadie más.
- **Credenciales para el agente**: secretos que el usuario guarda con un nombre y que el agente usa en sus comandos.
- **Ver y controlar el navegador en vivo**: el usuario ve el navegador del agente mientras trabaja y puede operarlo.
- **Uso de la computadora**: el agente maneja el escritorio con capturas de pantalla, mouse y teclado, más allá de un navegador.
- **Dispositivo iOS**, **Dispositivo Android**: un teléfono real conectado a la computadora, no un simulador.
- **Enlaces para compartir chats**: un enlace que muestra un chat a otra persona.

<details>
<summary>Fuentes</summary>

- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools), [Automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations), [Secrets](https://github.com/OpenHands/OpenHands/blob/main/src/routes/secrets-settings.tsx), [Files](https://github.com/OpenHands/OpenHands/blob/main/src/routes/files-tab.tsx), [Workspace download](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation-panel/conversation-card/conversation-card-context-menu.tsx), [Browser panel](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/browser/browser.tsx), [Public share](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation/conversation-name-context-menu.tsx), [Dictation](https://github.com/OpenHands/OpenHands/blob/main/src/hooks/chat/use-dictation.ts), [Worktrees](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/chat/workspace-mode-selector.tsx), [Usage](https://github.com/OpenHands/OpenHands/blob/main/src/routes/usage-tab.tsx), [Plugins](https://github.com/OpenHands/OpenHands/blob/main/src/routes/skills-plugins.tsx)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview), [File explorer](https://www.onorca.dev/docs/editing/file-explorer), [Editor](https://www.onorca.dev/docs/editing/monaco), [Session history](https://www.onorca.dev/docs/agents/session-history), [Worktrees](https://www.onorca.dev/docs/model/worktrees), [Annotate AI diffs](https://www.onorca.dev/docs/review/annotate-ai-diff), [SSH and port forwarding](https://www.onorca.dev/docs/ssh), [Automations](https://www.onorca.dev/docs/cli/automations), [Usage tracking](https://www.onorca.dev/docs/agents/usage-tracking), [Settings and plugins](https://www.onorca.dev/docs/settings), [iOS emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator/SKILL.md), [Android emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator-android/SKILL.md), [Client creation policy](https://github.com/stablyai/orca/blob/main/src/renderer/src/lib/client-creation-action-policy.ts)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md), [Usage and limits](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md), [Composer](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md), [Mobile notifications](https://github.com/pingdotgg/t3code/blob/main/docs/user/mobile-notifications.md), [Scheduled tasks](https://github.com/pingdotgg/t3code/blob/main/docs/user/project-settings.md), [Guest links](https://github.com/pingdotgg/t3code/blob/main/docs/user/guest-links.md), [Thread sidebar](https://github.com/pingdotgg/t3code/blob/main/docs/user/thread-sidebar.md), [Thread search](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ThreadSearch.ts), [File browser](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/FileBrowserPanel.tsx), [File saving](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/fileSaveCoordinator.ts), [Content search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/search/ProjectContentSearchDialog.tsx)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md), [Schedule](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/schedule.md), [Files sidebar](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-sidebar-files/README.md), [Workspace](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-workspace/README.md), [Plugin indexes](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/plugin-market.md), [Account quota](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-settings-account/src/client/AccountQuotaNotice.tsx), [Voice input](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/voice-input-bundle/README.md), [Liveshare](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/liveshare-protocol/README.md), [Worktree](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/worktree/README.md)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/), [GitHub](https://opencode.ai/docs/github/), [Plugins](https://opencode.ai/docs/plugins/), [Files](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/files/model.tsx), [Workspaces](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/submenu.tsx), [Session search](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/home/sessions/search.ts), [Review comments](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/composer/comment-note.ts), [AI library](https://github.com/anomalyco/opencode/blob/dev/packages/ai/README.md)
- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md), [Extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md), [Codemode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/codemode.md), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/), [Configuration](https://herdr.dev/docs/configuration/), [Plugins](https://herdr.dev/docs/plugins/)
- Codex: [Image generation skill](https://github.com/openai/codex/blob/main/codex-rs/skills/src/assets/samples/imagegen/SKILL.md)

</details>
