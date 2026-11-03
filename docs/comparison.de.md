# Demi im Vergleich

[English](comparison.md) · **Deutsch** · [Español (Latinoamérica)](comparison.es.md) · [Français](comparison.fr.md) · [Português (Brasil)](comparison.pt-BR.md) · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

Wie sich Demi von Agenten-Apps seiner Art unterscheidet, Stand Oktober 2026, nach der Dokumentation und dem Quellcode jedes Produkts. Die Tabelle führt eine Zeile nur dort auf, wo sich die Produkte unterscheiden, und eine Fähigkeit, die noch keines von ihnen hat. Ihre Spalten reichen vom Produkt, das Demi am nächsten ist, bis zum entferntesten.

Die Tabelle verwendet diese Symbole und Bezeichnungen:

- ✓ Unterstützt
- ◐ Teilweise
- Leer: nicht unterstützt
- **Web**: im Browser verwendet, ohne eigene App
- **PWA**: aus dem Browser als App installiert

Die folgende Tabelle vergleicht die Produkte Funktion für Funktion:

| | Demi | [OpenHands][openhands] | [Orca][orca] | [T3 Code][t3-code] | [DeepSeek Harness][deepseek-harness] | [opencode][opencode] | [Pi][pi] | [herdr][herdr] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Modelle und Abos** | | | | | | | | |
| Claude-Abo | ✓ | ✓ | ✓ | ✓ | ◐¹ | | ✓ | ✓ |
| ChatGPT, Grok und andere | ✓ | ✓ | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ |
| Mehrere Konten | ✓ | | ✓ | ✓ | | ✓ | | |
| Kontorotation | ✓ | | | | | | | |
| Abo-Nutzung | ✓ | ◐² | ✓ | ✓ | ◐³ | | | |
| Modellwechsel während einer Aufgabe | ✓ | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| **Oberfläche** | | | | | | | | |
| Web-App | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Mobil | PWA | Web | ✓ | ✓ | Web | Web | | |
| Desktop-App | PWA | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Terminal-Oberfläche | | ✓ | | | | ✓ | ✓ | ✓ |
| Spracheingabe | | ✓ | ✓ | ✓ | ◐⁴ | | | |
| Benachrichtigungen aufs Handy | ✓ | | ✓ | ✓⁵ | | | | ◐⁶ |
| Chat-Suche | ✓ | | ◐⁷ | ✓ | ◐⁸ | ◐⁸ | ✓ | |
| **Ausführung** | | | | | | | | |
| Zentraler Server | ✓ | ◐ | ◐ | | ◐ | ◐ | | |
| Läuft bei ausgeschalteten Geräten | ✓ | ✓⁹ | ◐ | ◐ | ◐ | ◐ | | ◐ |
| Synchronisierte Chats | ✓ | ◐ | ◐ | ◐ | | ◐ | | |
| Cloud-Sandbox | ✓ | ✓ | ◐ | | | | | |
| Entfernte Rechner | ✓ | ◐ | ✓ | ✓ | ◐ | ◐ | ◐ | ✓ |
| Aufgaben auf mehreren Rechnern | ✓ | | | | | | | |
| Rechnerwechsel | ✓ | | | | | | ◐ | |
| Portweiterleitung | ✓ | | ✓¹⁰ | | | | | |
| Geplante Aufgaben | ✓ | ✓ | ✓ | ✓ | ✓ | ◐¹¹ | | |
| **Dateien** | | | | | | | | |
| Dateibrowser | ✓ | ✓ | ✓ | ✓ | ◐¹² | ✓ | | |
| Dateiverwaltung | ◐¹³ | ◐¹⁴ | ✓ | ◐¹⁵ | | | | |
| Suche in Dateiinhalten | ✓ | | ✓ | ✓ | | | | |
| Git-Worktrees | ◐¹⁶ | ✓ | ✓ | ✓ | ◐⁴ | ✓ | | ✓ |
| Zeilenkommentare in Diffs | | | ✓ | ◐¹⁷ | | ✓ | | |
| **Agent** | | | | | | | | |
| Eigener Agent | ✓ | ✓ | | | ✓ | ✓ | ✓ | |
| Nur eine Shell als Werkzeug | ✓ | | | | | | | |
| Mehrere Agenten | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| Agentenprofile | ✓ | ✓ | | | ✓ | ✓ | ◐ | |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| MCP | | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| Plug-ins von Drittanbietern | | ✓ | ◐⁴ | | ✓ | ✓ | ✓ | ✓ |
| Zugangsdaten für den Agenten | ✓ | ✓ | | | | | | |
| Browserautomatisierung | ✓ | ✓ | ✓ | ✓ | ◐ | ◐¹⁸ | | |
| Browser live ansehen und steuern | ✓ | ◐¹⁹ | ✓ | ✓ | | ◐¹⁸ | | |
| Computersteuerung | | | ◐¹⁸ | ◐²⁰ | ◐⁴ | | | |
| **Mobilgeräte** | | | | | | | | |
| iOS-Simulator | ✓ | | ◐¹⁸ | ✓ | | | | |
| iOS-Gerät | | | | | | | | |
| Android-Emulator | ✓ | | ◐¹⁸ | ✓ | | | | |
| Android-Gerät | | | ◐¹⁸ | ✓ | | | | |
| **Mediengenerierung** | | | | | | | | |
| Bilder erstellen und bearbeiten | ✓ | ◐²¹ | ◐²¹ | ◐²¹ | ◐²¹ | | ◐²² | ◐²¹ |
| Videos erstellen | ✓ | | | | | | | |
| Sprache, Musik und Soundeffekte | ✓ | | | | | | | |
| **Teams** | | | | | | | | |
| Mehrere Benutzer | ✓ | ◐⁹ | | | | | | |
| Admin-Konsole | ✓ | ◐⁹ | | | | ◐⁹ | | |
| Isolierung pro Benutzer | ✓ | ◐⁹ | | | | | | |
| Links zu geteilten Chats | ✓ | ◐²³ | | ◐²⁴ | ◐⁴ | | ✓ | |

Die nummerierten Anmerkungen in der Tabelle besagen Folgendes:

1. Nur über die Subagenten Claude Code und Codex, die es startet. Die eigene Anmeldung per Abo steht im Code, hat aber keinen Eintrag in der App.
2. Eine Schätzung der Abo-Nutzung zu API-Preisen.
3. Nur das Kontingent des eigenen DeepSeek-Kontos.
4. Experimentell, von Hand eingeschaltet.
5. Nur über T3 Connect.
6. Nur über ein Beispiel-Plug-in, das an Telegram sendet.
7. Nach Titel, Ordner, Branch, Modell und Vorschau, nicht im ganzen Text.
8. Nur nach Titel.
9. Nur in der kostenpflichtigen Cloud- oder Enterprise-Edition.
10. Nur von einem SSH-Host zu diesem Computer; die lauschenden Ports des Hosts findet es selbst.
11. Nur als GitHub-Actions-Job nach einem Cron-Zeitplan.
12. Nur die Dateien des Rechners, auf dem es läuft; seine Webansichten erreichen keinen SSH-Host.
13. Hochladen, Herunterladen und neue Ordner; kein Umbenennen, Löschen, Verschieben oder Bearbeiten.
14. Nur das Herunterladen des ganzen Arbeitsbereichs.
15. Nur das Bearbeiten des Inhalts einer Datei.
16. Zeigt vorhandene Worktrees unter ihrem Projekt an, legt aber keinen an.
17. Ein Kommentar zu einer zitierten Stelle eines beliebigen Texts, nicht zu einer Zeile eines Diffs.
18. Nur auf dem Computer, auf dem seine Desktop-App läuft, nicht in seinem Web-Client oder auf einem entfernten Rechner.
19. Zeigt den letzten Screenshot des Agenten, der sich nicht bedienen lässt.
20. Nur iOS- und Android-Simulatoren, nicht der Desktop.
21. Über das Bildwerkzeug des Codex-Agenten, den es ausführt.
22. Nur aus einem Codemode-Skript, das von Hand eingeschaltet wird.
23. Nur mit einem OpenHands-Cloud-Backend.
24. Gastlinks, eine Beta, die T3 Connect braucht.

[openhands]: https://github.com/OpenHands/OpenHands
[orca]: https://github.com/stablyai/orca
[t3-code]: https://github.com/pingdotgg/t3code
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[opencode]: https://github.com/anomalyco/opencode
[pi]: https://github.com/earendil-works/pi
[herdr]: https://github.com/herdrdev/herdr

## Was die Zeilen bedeuten

Die Zeilen der Tabelle bedeuten Folgendes:

- **Zeilen zu einem Rechner** (Dateien, Worktrees, Diffs, der Browser, Computersteuerung und Mobilgeräte): Eine Funktion ist nur dann ✓, wenn sie auch funktioniert, während der Agent auf einem entfernten Rechner arbeitet, etwa einem SSH-Host, einem anderen Computer oder einem Rechner in der Cloud. Eine, die nur auf dem Computer funktioniert, auf dem das Produkt läuft, ist ◐.
- **Abos**: Anmelden mit einem Abo statt mit einem API-Schlüssel. Ein Abo, das über einen Agenten genutzt wird, den das Produkt ausführt, etwa Claude Code oder Codex, zählt.
- **Mehrere Konten**, **Kontorotation**: Erledigt vom Produkt selbst, nicht von einem Agenten, den es ausführt. Bei der Rotation geht die Arbeit an ein anderes Konto, sobald eines sein Limit erreicht.
- **Abo-Nutzung**: Wie viel der Limits jedes Abos verbraucht ist und wann jedes Limit zurückgesetzt wird.
- **Benachrichtigungen aufs Handy**: Eine Benachrichtigung erreicht das Handy, während die App geschlossen ist.
- **Chat-Suche**: Findet beliebige Wörter jeder Nachricht in allen Chats.
- **Zentraler Server**: Ein Server hält den Agenten und jeden Chat, und deine Computer führen nur seine Befehle aus.
  - ◐: Ein Server, der Befehle nur auf seinem eigenen Rechner ausführt.
  - Leer: Jeder Rechner führt einen eigenen Agenten aus, oder es gibt keinen Server.
- **Läuft bei ausgeschalteten Geräten**: Eine Aufgabe läuft weiter, während deine Computer und dein Handy aus sind.
  - ◐: Nur wenn du es auf einem eigenen Rechner installierst, der eingeschaltet bleibt.
- **Synchronisierte Chats**: Jeder Chat steht in einer Liste und sieht auf jedem Gerät gleich aus, egal auf welchem Rechner er läuft.
  - ◐: Nur zwischen den Geräten synchronisiert, die mit einem Server verbunden sind.
- **Aufgaben auf mehreren Rechnern**: Ein Chat arbeitet auf mehreren Rechnern.
- **Rechnerwechsel**: Ein Chat wechselt auf einen anderen Rechner und macht dort weiter.
- **Portweiterleitung**: Ein Programm auf einem Rechner erreicht einen Port eines anderen, wie es `ssh -L` ermöglicht.
- **Geplante Aufgaben**: Eine Aufgabe startet von selbst zu einer festgelegten Zeit oder in einem festen Intervall.
- **Dateiverwaltung**: Dateien in der App hochladen, herunterladen, anlegen, umbenennen, löschen, verschieben und bearbeiten.
- **Zeilenkommentare in Diffs**: Ein Kommentar zu einer Zeile einer Änderung, der an den Agenten geht.
- **Eigener Agent**: Der eigene Agent des Produkts steuert das Modell, statt Claude Code, Codex oder andere Agenten auszuführen.
- **Nur eine Shell als Werkzeug**: Das Modell bekommt ein einziges Werkzeug, eine Shell, und jede Fähigkeit ist ein Befehl darin.
- **Plug-ins von Drittanbietern**: Code von jemand anderem, den der Benutzer installiert, um dem Produkt Werkzeuge oder Ansichten hinzuzufügen, über Skills und MCP-Server hinaus. Demis eigene Funktionen sind Plug-ins, aber es lädt keine fremden.
- **Zugangsdaten für den Agenten**: Geheimnisse, die der Benutzer unter einem Namen speichert und die der Agent in seinen Befehlen verwendet.
- **Browser live ansehen und steuern**: Der Benutzer sieht dem Browser des Agenten bei der Arbeit zu und kann ihn bedienen.
- **Computersteuerung**: Der Agent bedient den Desktop mit Screenshots, Maus und Tastatur, über einen Browser hinaus.
- **iOS-Gerät**, **Android-Gerät**: Ein echtes Handy, das mit dem Computer verbunden ist, kein Simulator.
- **Links zu geteilten Chats**: Ein Link, der einen Chat jemand anderem zeigt.

<details>
<summary>Quellen</summary>

- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools), [Automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations), [Secrets](https://github.com/OpenHands/OpenHands/blob/main/src/routes/secrets-settings.tsx), [Files](https://github.com/OpenHands/OpenHands/blob/main/src/routes/files-tab.tsx), [Workspace download](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation-panel/conversation-card/conversation-card-context-menu.tsx), [Browser panel](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/browser/browser.tsx), [Public share](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation/conversation-name-context-menu.tsx), [Dictation](https://github.com/OpenHands/OpenHands/blob/main/src/hooks/chat/use-dictation.ts), [Worktrees](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/chat/workspace-mode-selector.tsx), [Usage](https://github.com/OpenHands/OpenHands/blob/main/src/routes/usage-tab.tsx), [Plugins](https://github.com/OpenHands/OpenHands/blob/main/src/routes/skills-plugins.tsx)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview), [File explorer](https://www.onorca.dev/docs/editing/file-explorer), [Editor](https://www.onorca.dev/docs/editing/monaco), [Session history](https://www.onorca.dev/docs/agents/session-history), [Worktrees](https://www.onorca.dev/docs/model/worktrees), [Annotate AI diffs](https://www.onorca.dev/docs/review/annotate-ai-diff), [SSH and port forwarding](https://www.onorca.dev/docs/ssh), [Automations](https://www.onorca.dev/docs/cli/automations), [Usage tracking](https://www.onorca.dev/docs/agents/usage-tracking), [Settings and plugins](https://www.onorca.dev/docs/settings), [iOS emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator/SKILL.md), [Android emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator-android/SKILL.md), [Client creation policy](https://github.com/stablyai/orca/blob/main/src/renderer/src/lib/client-creation-action-policy.ts)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md), [Usage and limits](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md), [Composer](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md), [Mobile notifications](https://github.com/pingdotgg/t3code/blob/main/docs/user/mobile-notifications.md), [Scheduled tasks](https://github.com/pingdotgg/t3code/blob/main/docs/user/project-settings.md), [Guest links](https://github.com/pingdotgg/t3code/blob/main/docs/user/guest-links.md), [Thread sidebar](https://github.com/pingdotgg/t3code/blob/main/docs/user/thread-sidebar.md), [Thread search](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ThreadSearch.ts), [File browser](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/FileBrowserPanel.tsx), [File saving](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/fileSaveCoordinator.ts), [Content search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/search/ProjectContentSearchDialog.tsx)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md), [Schedule](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/schedule.md), [Files sidebar](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-sidebar-files/README.md), [Workspace](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-workspace/README.md), [Plugin indexes](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/plugin-market.md), [Account quota](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-settings-account/src/client/AccountQuotaNotice.tsx), [Voice input](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/voice-input-bundle/README.md), [Liveshare](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/liveshare-protocol/README.md), [Worktree](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/worktree/README.md)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/), [GitHub](https://opencode.ai/docs/github/), [Plugins](https://opencode.ai/docs/plugins/), [Files](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/files/model.tsx), [Workspaces](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/submenu.tsx), [Session search](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/home/sessions/search.ts), [Review comments](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/composer/comment-note.ts), [AI library](https://github.com/anomalyco/opencode/blob/dev/packages/ai/README.md)
- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md), [Extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md), [Codemode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/codemode.md), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/), [Configuration](https://herdr.dev/docs/configuration/), [Plugins](https://herdr.dev/docs/plugins/)
- Codex: [Image generation skill](https://github.com/openai/codex/blob/main/codex-rs/skills/src/assets/samples/imagegen/SKILL.md)

</details>
