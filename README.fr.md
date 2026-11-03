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
  <a href="#licence"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0%20%2B%20Demi%20License-blue"></a>
  <a href="https://github.com/wspl/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/wspl/demi"></a>
  <a href="https://github.com/wspl/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/wspl/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.de.md">Deutsch</a> · <a href="README.es.md">Español (Latinoamérica)</a> · <b>Français</b> · <a href="README.pt-BR.md">Português (Brasil)</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="Demi dans un navigateur : un agent crée une mise en page sur deux colonnes pour une app iPhone, avec un commentaire et une question annexe sur sa réponse et des réponses suggérées en dessous, l’app sur un iPhone Duo à moitié plié dans le panneau latéral, et la même conversation sur un téléphone." width="800">
</p>

Demi est un agent IA web que vous hébergez sur votre propre serveur. Ouvrez-le
dans n’importe quel navigateur. Posez-lui une question ou confiez-lui un
travail : recherche, utilisation de sites web, manipulation de fichiers,
écriture de code, le tout dans la même conversation.

Chaque conversation se trouve sur votre serveur, au même endroit pour tous vos
appareils. Vos ordinateurs se contentent d’exécuter le travail : l’agent
travaille dans un bac à sable cloud ou sur vos propres machines, et les tâches
continuent quand vous fermez l’onglet.

<p align="center">
  <img src="docs/assets/concept.webp" alt="Depuis un ordinateur ou un téléphone, vous accédez à un serveur Demi, qui conserve toutes les conversations et travaille par l’intermédiaire de runners sur votre portable, votre ordinateur de bureau, votre HomeLab et votre VPS, ou dans des bacs à sable cloud." width="800">
</p>

> [!NOTE]
> Demi en est au début de son développement. Attendez-vous à des changements
> incompatibles entre les versions.

## Démarrage rapide

Il vous faut :

- Un serveur Linux sur lequel vous avez les droits root, avec l’un de ces
  systèmes ([distributions prises en
  charge](docs/delivery/installation.md#distributions)) :
  - Ubuntu 22.04 ou ultérieur, Debian 12 ou ultérieur
  - Fedora, RHEL 10, Rocky Linux, AlmaLinux
  - Arch Linux
- Un nom de domaine servi en HTTPS devant le serveur ([configurer
  HTTPS](docs/delivery/installation.md#https)).

Exécutez la commande sur le serveur. Elle installe Beta, les versions que Demi
publie ; `--canary` installe Canary, qui reçoit chaque build en premier :

```sh
# Beta
curl -fsSL https://demi.sh/install.sh | sudo bash
# Canary
curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --canary
```

Ou donnez ce prompt à un agent de programmation comme Claude Code ou Codex :

```text
Installe Demi sur mon serveur en suivant https://raw.githubusercontent.com/wspl/demi/main/docs/delivery/install-with-an-agent.md
```

Le programme d’installation pose quelques questions et configure Demi. Une fois
terminé :

1. Ouvrez votre domaine sans attendre et créez le compte administrateur.
2. Allez dans **Paramètres › Fournisseurs** et connectez-vous avec un abonnement
   ou ajoutez une clé API.
3. Démarrez une conversation. Elle s’exécute dans votre bac à sable cloud.
4. Pour travailler sur votre propre ordinateur, choisissez **Ajouter un
   appareil** dans le menu d’hôte de la conversation et exécutez la commande
   affichée sur cet ordinateur.

Consultez le [guide d’installation](docs/delivery/installation.md) pour toutes
les options, y compris l’installation sans questions, et les [mises à
niveau](docs/delivery/upgrades.md) pour passer à une nouvelle version.

## Fonctionnalités clés

Les fonctionnalités de Demi se répartissent en quatre groupes.

### Discuter, travailler et coder

- **Une conversation pour tout** : Posez une question rapide ou confiez un
  travail en plusieurs étapes au même endroit. Aucun mode à changer.
- **Programmation** : Modifiez des dépôts, lancez des builds et des tests, et
  relisez chaque modification sous forme de diff.
- **Recherche** : Cherchez sur le web et lisez des pages, des PDF et des
  documents.
- **Vrai navigateur** : Demi pilote Chrome pour se connecter, remplir des
  formulaires et télécharger des fichiers. Regardez-le dans le panneau latéral
  et prenez la main à tout moment.
- **Apps mobiles** : Demi pilote des simulateurs iOS et des émulateurs Android
  pour installer, lancer et tester une app, et enregistre l’écran. Regardez-les
  dans le panneau latéral et prenez la main à tout moment.
- **Fichiers et données** : Travaillez sur des documents, des feuilles de calcul
  et des images, et récupérez les résultats en pièces jointes.
- **Images, vidéo et audio** : Créez et modifiez des images, et générez des
  vidéos, de la voix, de la musique et des effets sonores avec les fournisseurs
  de modèles que vous ajoutez.
- **Partage** : Partagez une conversation, ou une seule réponse, sous forme de
  lien en lecture seule.
- **Commentaires et questions** : Sélectionnez des mots dans une réponse pour
  les commenter ou poser une question à leur sujet, sans interrompre la tâche.
- **Recherche** : Trouvez n’importe quel message dans toutes les conversations,
  ou dans celle que vous lisez.

### Partout, sur n’importe quelle machine

- **Fonctionne dans tout navigateur** : Ordinateur, tablette ou téléphone, sans
  rien installer. Ajoutez-le à votre écran d’accueil, et il s’adapte aux
  téléphones, tablettes et appareils pliables.
- **Neuf langues** : Anglais, allemand, espagnol, français, portugais, chinois
  simplifié, chinois traditionnel, japonais et coréen.
- **Toujours actif** : Les tâches continuent sur le serveur après la fermeture
  de l’onglet.
- **Notifications** : Soyez averti sur votre téléphone et vos ordinateurs quand
  une tâche se termine ou a besoin de vous, même quand Demi est fermé.
- **Un seul endroit pour toutes les conversations** : Le serveur stocke les
  conversations au lieu de les disperser sur les machines où l’agent a
  travaillé. Commencez sur votre portable et continuez sur votre téléphone, en
  direct. Changez de fournisseur d’IA et gardez votre historique.
- **Connexion par code QR et clé d’accès** : Scannez un code pour vous connecter
  sur votre téléphone, ou utilisez Touch ID, Face ID ou Windows Hello.
- **Bac à sable cloud privé** : Chaque utilisateur dispose de son propre bac à
  sable Linux sur votre serveur. Il s’exécute sous gVisor, ne peut atteindre ni
  les services de votre serveur ni votre réseau privé, et ne voit pas les bacs à
  sable des autres utilisateurs.
- **Vos propres ordinateurs** : Connectez des machines macOS, Windows et Linux
  avec une seule commande. Aucun port à ouvrir.
- **Tâches sur plusieurs machines** : Utilisez plusieurs machines dans une
  conversation, par exemple pour compiler sur un Mac et tester sous Windows.
- **Changer de machine en cours de tâche** : Déplacez une conversation entre le
  cloud et vos ordinateurs sans repartir de zéro.
- **Des fichiers sur chaque machine** : Parcourez les fichiers du bac à sable et
  de chacun de vos ordinateurs, prévisualisez images, vidéos, audio, PDF et
  Markdown, envoyez et téléchargez des fichiers, et recherchez dans les noms et
  le texte.
- **Redirection de port** : Atteignez un port d’une machine depuis une autre.

### Agent

- **Plusieurs agents** : Demi répartit une grande tâche entre une équipe
  d’agents qui travaillent en parallèle, puis combine leurs résultats.
- **Profils d’agent** : Définissez des agents spécialisés, par exemple pour la
  recherche ou la relecture, chacun avec son propre modèle, son niveau de
  raisonnement, ses instructions et ses commandes.
- **Flux de travail** : Regroupez un ensemble de profils d’agent avec des
  instructions pour un type de travail. Placez les flux de travail dans le
  dossier `.demi/workflows/` d’un projet, choisissez-en un dans la zone de
  saisie et partagez-les avec votre équipe via le dépôt.
- **Planifications** : Réveillez une conversation une fois, selon un cycle
  répété, à intervalles, avec cron, quand la sortie d’une commande change ou
  quand une URL est appelée.
- **Les conversations collaborent** : Une conversation envoie des messages à une
  autre et lit l’historique d’autres conversations.
- **Compétences** : Installez des compétences pour apprendre de nouvelles tâches
  à Demi.
- **Identifiants sécurisés** : Demi demande les clés API et les mots de passe
  via un formulaire sécurisé, et ils n’apparaissent jamais dans la conversation.

### Modèles et équipes

- **Utilisez vos abonnements** : Connectez-vous avec des comptes Claude, ChatGPT
  ou Grok Build. Ajoutez plusieurs comptes par abonnement, et Demi alterne entre
  eux à mesure que leurs limites d’utilisation s’épuisent.
- **Forfaits de programmation** : Utilisez la clé d’un forfait de programmation,
  comme Kimi, GLM ou MiniMax.
- **Clés API** : Anthropic, OpenAI, Gemini ou tout point de terminaison
  compatible.
- **Modèles locaux** : Laissez votre serveur utiliser un modèle qui tourne sur
  votre propre ordinateur, comme Ollama, grâce à la redirection de port.
- **Changer de modèle à tout moment** : Changez de modèle ou de niveau de
  raisonnement, même en cours de tâche.
- **Utilisation et coût** : Voyez la part utilisée des limites de chaque
  abonnement et quand elles se réinitialisent, et ce que le travail a coûté sur
  la dernière heure, le dernier jour ou depuis le début.
- **Plusieurs utilisateurs** : Chaque utilisateur a son propre bac à sable, ses
  appareils et ses conversations, avec les abonnements de l’administrateur ou
  les siens.
- **Outils d’administration** : Ajoutez des utilisateurs, gérez les rôles,
  réinitialisez les mots de passe et envoyez des codes de connexion pour le
  téléphone.
- **Installation et mises à niveau en une ligne** : Configurez un serveur avec
  une commande et mettez-le à niveau avec une autre.

## Un agent bâti sur un shell

Demi est à la fois l’app et l’agent.

Ce n’est pas une surcouche de Claude Code ou de Codex. Son propre runtime pilote
chaque modèle, et un abonnement Claude ou ChatGPT ne fournit que le modèle.

La plupart des agents donnent au modèle une longue liste d’outils. Demi lui en
donne un seul : un shell. Tout le reste, du navigateur aux autres machines en
passant par les sous-agents, est une commande `demi` dans ce shell :

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

Cette conception a les avantages suivants :

- **Composable** : Les commandes s’enchaînent avec des pipes et `&&`, si bien
  que l’agent fait en une étape ce qui demande d’ordinaire plusieurs appels
  d’outils.
- **Prompts légers** : Le modèle voit une ligne par groupe de commandes et lit
  `--help` quand il lui en faut plus. Les nouvelles capacités n’alourdissent pas
  chaque requête.
- **Le même shell partout** : Chaque machine exécute le même shell compatible
  bash, écrit en Rust ([brush](https://github.com/reubeno/brush)), si bien qu’un
  script qui fonctionne dans le cloud fonctionne aussi sur macOS et Windows. Les
  outils qui modifient des fichiers sont ceux de GNU sur chaque machine
  ([uutils](https://github.com/uutils/coreutils)), donc leurs modifications sont
  enregistrées ; tout autre programme, comme `tar` ou `python`, est celui du
  système et ses modifications ne sont pas enregistrées. Sous Windows, les
  autres utilitaires sont ceux de Git for Windows, que l’installation de Demi
  ajoute s’il manque.
- **Pareil pour tous les modèles** : Chaque fournisseur voit le même outil
  unique, donc changer de modèle ne change pas ce que l’agent peut faire.
- **Conçu pour les longues tâches** : Les commandes continuent en arrière-plan
  et rendent compte quand elles se terminent, si bien que les serveurs de
  développement et les suites de tests ne bloquent jamais la conversation.

> [!NOTE]
> **Prévu** : une OpenAPI pour le serveur Demi, afin que vos propres programmes
> puissent utiliser ses conversations, ses agents et ses bacs à sable cloud pour
> les agents dont vous avez besoin.

## Architecture

Le schéma suivant montre comment les parties s’articulent :

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
  │ (gVisor)       │  │ Linux)                       │
  └────────────────┘  └──────────────────────────────┘
```

Cette conception a les propriétés suivantes :

- **Le serveur exécute l’agent, les runners font le travail** : Les
  conversations, les agents et les paramètres résident sur le serveur. Un petit
  runner sur chaque machine ne s’occupe que des fichiers, des commandes et du
  navigateur. Il se connecte au serveur de l’intérieur vers l’extérieur, donc il
  fonctionne derrière un NAT et des pare-feu.
- **Un seul runner partout** : Le bac à sable cloud et vos ordinateurs exécutent
  le même runner, donc chaque fonctionnalité marche sur chaque machine.
- **Économe en bande passante** : Les sorties et les fichiers volumineux restent
  sur la machine qui les a produits. Le navigateur ne les charge que lorsque
  vous les ouvrez.
- **Natif** : Le serveur, le runner et les outils de l’agent sont écrits en
  Rust. L’app web est construite avec Vue et TypeScript.

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

La majeure partie de Demi est publiée sous la [Apache License 2.0](LICENSE).
Vous pouvez utiliser ces parties pour tout usage :

- Le runtime de l’agent.
- Les fournisseurs de modèles, et la bibliothèque qui les configure à partir de
  votre configuration.
- Le runner que vous installez sur vos machines, et la bibliothèque qui le
  pilote.
- Les commandes.
- Les plug-ins, leur SDK et l’hôte qui les exécute.
- La bibliothèque de composants d’interface et sa galerie.
- Les protocoles.
- La documentation.

Le serveur et l’app web (`crates/backend*`, `crates/server`,
`crates/machine-manager` et `packages/web`) sont publiés sous la [Demi
License](LICENSE-DEMI), qui est la Apache License 2.0 avec des conditions
supplémentaires :

- **Libre d’utilisation** : Utilisez-les, modifiez-les et exécutez-les, y
  compris dans votre entreprise pour votre propre équipe.
- **Libre d’assistance** : Facturez le déploiement, l’intégration ou
  l’assistance d’instances de Demi que d’autres exploitent eux-mêmes.
- **Pas de revente en tant que service** : Ne les proposez pas à d’autres contre
  rémunération comme service hébergé ou multilocataire, ni comme produit qui est
  essentiellement Demi.
- **Conservez la marque** : Ne supprimez pas et ne modifiez pas le nom, le logo
  ni les mentions de copyright de Demi dans l’app web.

Pour d’autres usages, contactez [support@demi.sh](mailto:support@demi.sh).
