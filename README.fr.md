<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>Votre agent, sur toutes vos machines.<br>
  Un agent IA web que vous hébergez sur votre propre serveur.</strong>
</p>

<p align="center">
  <a href="#licence"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.de.md">Deutsch</a> · <a href="README.es.md">Español</a> · <b>Français</b> · <a href="README.pt-BR.md">Português</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="Demi dans un navigateur : un agent crée une mise en page sur deux colonnes pour une app iPhone, avec un commentaire et une question annexe sur sa réponse et des réponses suggérées en dessous, l’app sur un iPhone Duo à moitié plié dans le panneau latéral, et la même conversation sur un téléphone." width="800">
</p>

Demi est un agent IA web que vous hébergez sur votre propre serveur.
Ouvrez-le dans n’importe quel navigateur et confiez-lui tout : recherche, sites
web, fichiers ou code. Vos conversations restent sur votre serveur, et le
travail s’exécute dans un sandbox cloud ou sur vos propres machines, même
après la fermeture de l’onglet.

> [!NOTE]
> Demi en est aux premiers stades de développement. Attendez-vous à des
> changements incompatibles entre les versions.

## Démarrage rapide

Déployez Demi en quelques clics :

<p>
  <a href="docs/delivery/railway.md"><img alt="Deploy on Railway" src="https://railway.com/button.svg" height="40"></a>
  <a href="docs/delivery/fly.md"><img alt="Deploy on Fly.io" src="docs/assets/deploy/deploy-on-fly.svg" height="40"></a>
  <a href="docs/delivery/docker-compose.md"><img alt="Run with Docker Compose" src="docs/assets/deploy/run-with-docker-compose.svg" height="40"></a>
</p>

Ou installez-le sur votre propre serveur Linux, avec un nom de domaine :

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

Ou demandez à un agent de code, comme Claude Code ou Codex, de l’installer :

```text
Installe Demi sur mon serveur en suivant https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md
```

Ouvrez ensuite Demi, créez le compte administrateur et ajoutez un modèle dans
**Paramètres › Fournisseurs**. Le [guide d’installation](docs/delivery/installation.md)
couvre les systèmes pris en charge, HTTPS, le canal Canary et le coût de
chaque option.

<p align="center">
  <img src="docs/assets/concept.webp" alt="Depuis un ordinateur ou un téléphone, vous accédez à un serveur Demi, qui conserve toutes les conversations et travaille par l’intermédiaire de runners sur votre portable, votre ordinateur de bureau, votre HomeLab et votre VPS, ou dans des bacs à sable cloud." width="800">
</p>

## Fonctionnalités clés

### Discuter, travailler et coder

- **Une conversation pour tout** : questions rapides comme tâches en plusieurs étapes.
- **Code** : modifiez des dépôts, lancez builds et tests, relisez chaque changement sous forme de diff.
- **Recherche** : cherchez sur le web et lisez pages, PDF et documents.
- **Vrai navigateur** : Demi pilote Chrome. Regardez-le et reprenez la main à tout moment.
- **Apps mobiles** : installez et testez des apps sur simulateurs iOS et émulateurs Android.
- **Fichiers et données** : documents, tableurs et images, rendus en pièces jointes.
- **Images, vidéo et audio** : créés via les fournisseurs de modèles que vous ajoutez.
- **Commentaires et questions** : sélectionnez des mots d’une réponse pour poser une question.
- **Partage et recherche** : partagez un lien en lecture seule et cherchez dans chaque message.

### Partout, sur n’importe quelle machine

- **N’importe quel navigateur** : ordinateur, tablette, téléphone ou pliable, sans rien installer.
- **Toujours actif** : les tâches continuent après la fermeture de l’onglet et vous préviennent.
- **Un seul lieu pour les conversations** : commencez sur le portable, continuez sur le téléphone.
- **Sandbox cloud privé** : chaque utilisateur a son propre sandbox Linux.
- **Vos propres ordinateurs** : connectez macOS, Windows et Linux en une commande.
- **Plusieurs machines, une tâche** : compilez sur un Mac, testez sous Windows, changez en cours de route.
- **Fichiers partout** : parcourez, prévisualisez et cherchez les fichiers de chaque machine.
- **Redirection de ports** : atteignez un port d’une machine depuis une autre.
- **Connexion à votre façon** : code QR ou passkey, en neuf langues.

### Agent

- **Multi-agent** : une équipe d’agents découpe une grande tâche et travaille en parallèle.
- **Profils et workflows** : des agents spécialisés, partagés via votre dépôt.
- **Planifications** : réveillez une conversation par minuteur, par cron ou sur un changement.
- **Les conversations collaborent** : une conversation écrit à une autre ou la lit.
- **Skills** : apprenez de nouvelles tâches à Demi.
- **Identifiants sécurisés** : les secrets passent par un formulaire, jamais par la conversation.

### Modèles et équipes

- **Vos abonnements** : Claude, ChatGPT et Grok Build, avec rotation des comptes à leurs limites.
- **N’importe quelle API** : Anthropic, OpenAI, Gemini, offres de code comme Kimi, GLM ou MiniMax, et modèles locaux.
- **Changez de modèle à tout moment**, même en pleine tâche.
- **Usage et coût** : voyez les limites de chaque abonnement et ce qu’a coûté le travail.
- **Multi-utilisateur** : chaque utilisateur a son propre sandbox, ses appareils et ses conversations.

## Un agent bâti sur un shell

Demi n’encapsule ni Claude Code ni Codex : son propre runtime pilote chaque
modèle. Là où la plupart des agents donnent au modèle une longue liste
d’outils, Demi lui en donne un seul, un shell. Le navigateur, les autres
machines et les sous-agents sont des commandes `demi` dans ce shell :

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **Composable** : les commandes s’enchaînent par des pipes, une étape fait donc le travail de plusieurs appels d’outils.
- **Prompts légers** : le modèle lit le `--help` d’une commande seulement quand il en a besoin.
- **Le même shell partout** : un shell compatible bash, écrit en Rust, sur chaque machine.
- **Identique pour chaque modèle** : changer de modèle ne change pas ce que l’agent sait faire.
- **Conçu pour les longues tâches** : les commandes tournent en arrière-plan et rendent compte à la fin.

> [!NOTE]
> **Prévu** : une OpenAPI pour le serveur Demi, afin que vos propres
> programmes utilisent ses conversations, agents et sandboxes cloud.

## Architecture

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

- **Le serveur fait tourner l’agent, les runners font le travail** : chaque
  runner se connecte vers l’extérieur et fonctionne donc derrière un NAT ou un
  pare-feu.
- **Un seul runner partout** : chaque fonctionnalité marche sur chaque machine.
- **Économe en bande passante** : les gros fichiers restent là où ils ont été créés jusqu’à ce que vous les ouvriez.
- **Natif** : Rust pour le serveur, le runner et les outils ; Vue et TypeScript pour l’app web.

Lisez la [documentation de conception](docs/README.md), en commençant par la
[vue d’ensemble](docs/overview.md) (en anglais).

## Comparaison

Ce qui distingue Demi d’OpenHands, Orca, T3 Code, DeepSeek Harness, opencode, Pi
et herdr, fonctionnalité par fonctionnalité :
[Demi comparé](docs/comparison.fr.md).

## Documentation

- [Documentation de conception](docs/README.md) : le fonctionnement de chaque
  partie de Demi
- [Contribuer](CONTRIBUTING.md) : environnement de développement, vérifications
  et ajout d’un fournisseur
- [Sécurité](SECURITY.md) : signaler une vulnérabilité et où les données sont
  stockées

Ces documents sont en anglais.

## Licence

Demi est publié sous la [Apache License 2.0](LICENSE).
