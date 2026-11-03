# Demi comparé

[English](comparison.md) · [Deutsch](comparison.de.md) · [Español](comparison.es.md) · **Français** · [Português](comparison.pt-BR.md) · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

Ce qui distingue Demi des apps d’agent du même type, en octobre 2026, d’après la documentation et le code source de chaque produit. Le tableau ne contient une ligne que là où les produits diffèrent, plus une capacité qu’aucun d’eux n’a encore. Ses colonnes vont du produit le plus proche de Demi au plus éloigné.

Le tableau utilise ces symboles et libellés :

- ✓ Pris en charge
- ◐ En partie
- Vide : non pris en charge
- **Web** : utilisé dans le navigateur, sans app dédiée
- **PWA** : installé depuis le navigateur comme une app

Le tableau suivant compare les produits fonctionnalité par fonctionnalité :

| | Demi | [OpenHands][openhands] | [Orca][orca] | [T3 Code][t3-code] | [DeepSeek Harness][deepseek-harness] | [opencode][opencode] | [Pi][pi] | [herdr][herdr] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Modèles et abonnements** | | | | | | | | |
| Abonnement Claude | ✓ | ✓ | ✓ | ✓ | ◐¹ | | ✓ | ✓ |
| ChatGPT, Grok et autres | ✓ | ✓ | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ |
| Plusieurs comptes | ✓ | | ✓ | ✓ | | ✓ | | |
| Rotation des comptes | ✓ | | | | | | | |
| Utilisation de l’abonnement | ✓ | ◐² | ✓ | ✓ | ◐³ | | | |
| Changement de modèle en cours de tâche | ✓ | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| **Interface** | | | | | | | | |
| App web | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Mobile | PWA | Web | ✓ | ✓ | Web | Web | | |
| App de bureau | PWA | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Interface en terminal | | ✓ | | | | ✓ | ✓ | ✓ |
| Saisie vocale | | ✓ | ✓ | ✓ | ◐⁴ | | | |
| Notifications sur le téléphone | ✓ | | ✓ | ✓⁵ | | | | ◐⁶ |
| Recherche dans les conversations | ✓ | | ◐⁷ | ✓ | ◐⁸ | ◐⁸ | ✓ | |
| **Exécution** | | | | | | | | |
| Serveur central | ✓ | ◐ | ◐ | | ◐ | ◐ | | |
| Fonctionne avec tous les appareils éteints | ✓ | ✓⁹ | ◐ | ◐ | ◐ | ◐ | | ◐ |
| Conversations synchronisées | ✓ | ◐ | ◐ | ◐ | | ◐ | | |
| Bac à sable cloud | ✓ | ✓ | ◐ | | | | | |
| Machines distantes | ✓ | ◐ | ✓ | ✓ | ◐ | ◐ | ◐ | ✓ |
| Tâches sur plusieurs machines | ✓ | | | | | | | |
| Changement de machine | ✓ | | | | | | ◐ | |
| Redirection de port | ✓ | | ✓¹⁰ | | | | | |
| Tâches planifiées | ✓ | ✓ | ✓ | ✓ | ✓ | ◐¹¹ | | |
| **Fichiers** | | | | | | | | |
| Navigateur de fichiers | ✓ | ✓ | ✓ | ✓ | ◐¹² | ✓ | | |
| Gestion des fichiers | ◐¹³ | ◐¹⁴ | ✓ | ◐¹⁵ | | | | |
| Recherche dans le contenu des fichiers | ✓ | | ✓ | ✓ | | | | |
| Worktrees Git | ◐¹⁶ | ✓ | ✓ | ✓ | ◐⁴ | ✓ | | ✓ |
| Commentaires de ligne sur les diffs | | | ✓ | ◐¹⁷ | | ✓ | | |
| **Agent** | | | | | | | | |
| Agent propre | ✓ | ✓ | | | ✓ | ✓ | ✓ | |
| Un shell pour seul outil | ✓ | | | | | | | |
| Plusieurs agents | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| Profils d’agent | ✓ | ✓ | | | ✓ | ✓ | ◐ | |
| Compétences | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| MCP | | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| Plug-ins tiers | | ✓ | ◐⁴ | | ✓ | ✓ | ✓ | ✓ |
| Identifiants pour l’agent | ✓ | ✓ | | | | | | |
| Automatisation du navigateur | ✓ | ✓ | ✓ | ✓ | ◐ | ◐¹⁸ | | |
| Voir et piloter le navigateur en direct | ✓ | ◐¹⁹ | ✓ | ✓ | | ◐¹⁸ | | |
| Utilisation de l’ordinateur | | | ◐¹⁸ | ◐²⁰ | ◐⁴ | | | |
| **Appareils mobiles** | | | | | | | | |
| Simulateur iOS | ✓ | | ◐¹⁸ | ✓ | | | | |
| Appareil iOS | | | | | | | | |
| Émulateur Android | ✓ | | ◐¹⁸ | ✓ | | | | |
| Appareil Android | | | ◐¹⁸ | ✓ | | | | |
| **Génération de médias** | | | | | | | | |
| Génération et retouche d’images | ✓ | ◐²¹ | ◐²¹ | ◐²¹ | ◐²¹ | | ◐²² | ◐²¹ |
| Génération de vidéos | ✓ | | | | | | | |
| Voix, musique et effets sonores | ✓ | | | | | | | |
| **Équipes** | | | | | | | | |
| Plusieurs utilisateurs | ✓ | ◐⁹ | | | | | | |
| Console d’administration | ✓ | ◐⁹ | | | | ◐⁹ | | |
| Isolation par utilisateur | ✓ | ◐⁹ | | | | | | |
| Liens de partage de conversation | ✓ | ◐²³ | | ◐²⁴ | ◐⁴ | | ✓ | |

Les notes numérotées du tableau disent ce qui suit :

1. Uniquement via les sous-agents Claude Code et Codex qu’il lance. Sa propre connexion par abonnement figure dans le code, mais n’a pas d’entrée dans l’app.
2. Une estimation de l’utilisation du forfait aux tarifs de l’API.
3. Uniquement le quota de son propre compte DeepSeek.
4. Expérimental, à activer à la main.
5. Uniquement via T3 Connect.
6. Uniquement via un plug-in d’exemple qui envoie vers Telegram.
7. Par titre, dossier, branche, modèle et aperçu, pas dans le texte entier.
8. Par titre uniquement.
9. Uniquement dans l’édition cloud payante ou entreprise.
10. Uniquement d’un hôte SSH vers cet ordinateur ; il trouve pour vous les ports en écoute de l’hôte.
11. Uniquement comme tâche GitHub Actions planifiée par cron.
12. Uniquement les fichiers de la machine où il s’exécute ; ses vues web n’atteignent pas un hôte SSH.
13. Envoi, téléchargement et nouveaux dossiers ; pas de renommage, de suppression, de déplacement ni de modification.
14. Uniquement le téléchargement de tout l’espace de travail.
15. Uniquement la modification du contenu d’un fichier.
16. Affiche les worktrees existants sous leur projet, mais n’en crée pas.
17. Un commentaire sur un passage cité de n’importe quel texte, pas sur une ligne d’un diff.
18. Uniquement sur l’ordinateur qui exécute son app de bureau, pas dans son client web ni sur une machine distante.
19. Affiche la dernière capture d’écran de l’agent, qu’on ne peut pas manipuler.
20. Uniquement les simulateurs iOS et Android, pas le bureau.
21. Via l’outil d’images de l’agent Codex qu’il exécute.
22. Uniquement depuis un script codemode, à activer à la main.
23. Uniquement avec un backend OpenHands Cloud.
24. Liens invités, une bêta qui nécessite T3 Connect.

[openhands]: https://github.com/OpenHands/OpenHands
[orca]: https://github.com/stablyai/orca
[t3-code]: https://github.com/pingdotgg/t3code
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[opencode]: https://github.com/anomalyco/opencode
[pi]: https://github.com/earendil-works/pi
[herdr]: https://github.com/herdrdev/herdr

## Ce que signifient les lignes

Les lignes du tableau signifient ce qui suit :

- **Lignes liées à une machine** (fichiers, worktrees, diffs, navigateur, utilisation de l’ordinateur et appareils mobiles) : une fonctionnalité n’est ✓ que si elle fonctionne aussi pendant que l’agent travaille sur une machine distante, comme un hôte SSH, un autre ordinateur ou une machine cloud. Une fonctionnalité qui ne fonctionne que sur l’ordinateur qui exécute le produit est ◐.
- **Abonnements** : se connecter avec un abonnement au lieu d’une clé API. Un abonnement utilisé via un agent que le produit exécute, comme Claude Code ou Codex, compte.
- **Plusieurs comptes**, **Rotation des comptes** : assurés par le produit lui-même, pas par un agent qu’il exécute. La rotation transfère le travail vers un autre compte quand l’un atteint sa limite.
- **Utilisation de l’abonnement** : la part utilisée des limites de chaque abonnement, et quand chaque limite se réinitialise.
- **Notifications sur le téléphone** : une notification arrive sur le téléphone quand l’app est fermée.
- **Recherche dans les conversations** : trouve n’importe quels mots de n’importe quel message dans toutes les conversations.
- **Serveur central** : un serveur contient l’agent et toutes les conversations, et vos ordinateurs ne font qu’exécuter ses commandes.
  - ◐ : un serveur qui n’exécute des commandes que sur sa propre machine.
  - Vide : chaque machine exécute son propre agent, ou il n’y a pas de serveur.
- **Fonctionne avec tous les appareils éteints** : une tâche continue pendant que vos ordinateurs et votre téléphone sont éteints.
  - ◐ : seulement si vous l’installez sur une machine à vous qui reste allumée.
- **Conversations synchronisées** : toutes les conversations figurent dans une seule liste et s’affichent de la même façon sur chaque appareil, quelle que soit la machine où elles s’exécutent.
  - ◐ : synchronisées seulement entre les appareils connectés à un même serveur.
- **Tâches sur plusieurs machines** : une conversation travaille sur plusieurs machines.
- **Changement de machine** : une conversation passe sur une autre machine et continue.
- **Redirection de port** : un programme sur une machine atteint un port d’une autre, comme le permet `ssh -L`.
- **Tâches planifiées** : une tâche démarre d’elle-même à une heure fixée ou à intervalles réguliers.
- **Gestion des fichiers** : envoyer, télécharger, créer, renommer, supprimer, déplacer et modifier des fichiers dans l’app.
- **Commentaires de ligne sur les diffs** : un commentaire sur une ligne d’une modification, transmis à l’agent.
- **Agent propre** : l’agent du produit lui-même pilote le modèle, au lieu d’exécuter Claude Code, Codex ou d’autres agents.
- **Un shell pour seul outil** : le modèle reçoit un seul outil, un shell, et chaque capacité est une commande de ce shell.
- **Plug-ins tiers** : du code écrit par quelqu’un d’autre, que l’utilisateur installe pour ajouter des outils ou des écrans au produit, au-delà des compétences et des serveurs MCP. Les fonctionnalités de Demi sont elles-mêmes des plug-ins, mais il ne charge ceux de personne d’autre.
- **Identifiants pour l’agent** : des secrets que l’utilisateur enregistre sous un nom et que l’agent utilise dans ses commandes.
- **Voir et piloter le navigateur en direct** : l’utilisateur regarde le navigateur de l’agent pendant qu’il travaille et peut le manipuler.
- **Utilisation de l’ordinateur** : l’agent pilote le bureau avec des captures d’écran, la souris et le clavier, au-delà d’un navigateur.
- **Appareil iOS**, **Appareil Android** : un vrai téléphone connecté à l’ordinateur, pas un simulateur.
- **Liens de partage de conversation** : un lien qui montre une conversation à quelqu’un d’autre.

<details>
<summary>Sources</summary>

- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools), [Automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations), [Secrets](https://github.com/OpenHands/OpenHands/blob/main/src/routes/secrets-settings.tsx), [Files](https://github.com/OpenHands/OpenHands/blob/main/src/routes/files-tab.tsx), [Workspace download](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation-panel/conversation-card/conversation-card-context-menu.tsx), [Browser panel](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/browser/browser.tsx), [Public share](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation/conversation-name-context-menu.tsx), [Dictation](https://github.com/OpenHands/OpenHands/blob/main/src/hooks/chat/use-dictation.ts), [Worktrees](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/chat/workspace-mode-selector.tsx), [Usage](https://github.com/OpenHands/OpenHands/blob/main/src/routes/usage-tab.tsx), [Plugins](https://github.com/OpenHands/OpenHands/blob/main/src/routes/skills-plugins.tsx)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview), [File explorer](https://www.onorca.dev/docs/editing/file-explorer), [Editor](https://www.onorca.dev/docs/editing/monaco), [Session history](https://www.onorca.dev/docs/agents/session-history), [Worktrees](https://www.onorca.dev/docs/model/worktrees), [Annotate AI diffs](https://www.onorca.dev/docs/review/annotate-ai-diff), [SSH and port forwarding](https://www.onorca.dev/docs/ssh), [Automations](https://www.onorca.dev/docs/cli/automations), [Usage tracking](https://www.onorca.dev/docs/agents/usage-tracking), [Settings and plugins](https://www.onorca.dev/docs/settings), [iOS emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator/SKILL.md), [Android emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator-android/SKILL.md), [Client creation policy](https://github.com/stablyai/orca/blob/main/src/renderer/src/lib/client-creation-action-policy.ts)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md), [Usage and limits](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md), [Composer](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md), [Mobile notifications](https://github.com/pingdotgg/t3code/blob/main/docs/user/mobile-notifications.md), [Scheduled tasks](https://github.com/pingdotgg/t3code/blob/main/docs/user/project-settings.md), [Guest links](https://github.com/pingdotgg/t3code/blob/main/docs/user/guest-links.md), [Thread sidebar](https://github.com/pingdotgg/t3code/blob/main/docs/user/thread-sidebar.md), [Thread search](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ThreadSearch.ts), [File browser](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/FileBrowserPanel.tsx), [File saving](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/fileSaveCoordinator.ts), [Content search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/search/ProjectContentSearchDialog.tsx)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md), [Schedule](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/schedule.md), [Files sidebar](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-sidebar-files/README.md), [Workspace](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-workspace/README.md), [Plugin indexes](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/plugin-market.md), [Account quota](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-settings-account/src/client/AccountQuotaNotice.tsx), [Voice input](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/voice-input-bundle/README.md), [Liveshare](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/liveshare-protocol/README.md), [Worktree](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/worktree/README.md)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/), [GitHub](https://opencode.ai/docs/github/), [Plugins](https://opencode.ai/docs/plugins/), [Files](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/files/model.tsx), [Workspaces](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/submenu.tsx), [Session search](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/home/sessions/search.ts), [Review comments](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/composer/comment-note.ts), [AI library](https://github.com/anomalyco/opencode/blob/dev/packages/ai/README.md)
- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md), [Extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md), [Codemode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/codemode.md), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/), [Configuration](https://herdr.dev/docs/configuration/), [Plugins](https://herdr.dev/docs/plugins/)
- Codex: [Image generation skill](https://github.com/openai/codex/blob/main/codex-rs/skills/src/assets/samples/imagegen/SKILL.md)

</details>
