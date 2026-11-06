<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>Dein Agent, auf all deinen Rechnern.<br>
  Ein webbasierter KI-Agent, den du auf deinem eigenen Server hostest.</strong>
</p>

<p align="center">
  <a href="#lizenz"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <b>Deutsch</b> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <a href="README.pt-BR.md">Português</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="Demi im Browser: Ein Agent baut ein zweispaltiges Layout für eine iPhone-App, mit einem Kommentar und einer Nebenfrage zu seiner Antwort und Antwortvorschlägen darunter, die App auf einem halb gefalteten iPhone Duo im Seitenpanel und derselbe Chat auf einem Telefon." width="800">
</p>

Demi ist ein webbasierter KI-Agent, den du auf deinem eigenen Server
betreibst. Öffne ihn in jedem Browser und gib ihm alles: Recherche, Websites,
Dateien oder Code. Deine Unterhaltungen bleiben auf deinem Server, und die
Arbeit läuft in einer Cloud-Sandbox oder auf deinen eigenen Rechnern weiter,
auch wenn du den Tab schließt.

> [!NOTE]
> Demi befindet sich in einer frühen Entwicklungsphase. Zwischen Releases
> sind inkompatible Änderungen zu erwarten.

## Schnellstart

Demi mit wenigen Klicks bereitstellen:

<p>
  <a href="docs/delivery/railway.md"><img alt="Deploy on Railway" src="https://railway.com/button.svg" height="36"></a>
  <a href="https://render.com/deploy?repo=https://github.com/demi-sh/demi"><img alt="Deploy on Render" src="docs/assets/deploy/deploy-on-render.svg" height="36"></a>
  <a href="docs/delivery/fly.md"><img alt="Deploy on Fly.io" src="docs/assets/deploy/deploy-on-fly.svg" height="36"></a>
  <a href="docs/delivery/docker-compose.md"><img alt="Run with Docker Compose" src="docs/assets/deploy/run-with-docker-compose.svg" height="36"></a>
</p>

Oder auf einem eigenen Linux-Server mit Domainnamen installieren:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

Oder einen Coding-Agenten wie Claude Code oder Codex die Installation
übernehmen lassen:

```text
Installiere Demi auf meinem Server nach dieser Anleitung: https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md
```

Öffne danach Demi, lege das Admin-Konto an und füge unter
**Einstellungen › Anbieter** ein Modell hinzu. Die
[Installationsanleitung](docs/delivery/installation.md) beschreibt
unterstützte Systeme, HTTPS, den Canary-Kanal und die Kosten jeder Variante.

<p align="center">
  <img src="docs/assets/concept.webp" alt="Von einem Computer oder einem Telefon erreichst du einen Demi-Server, der jeden Chat speichert und über Runner auf deinem Laptop, Desktop, HomeLab und VPS oder in Cloud-Sandboxes arbeitet." width="800">
</p>

## Wichtige Funktionen

### Chatten, arbeiten und programmieren

- **Eine Unterhaltung für alles**: kurze Fragen und mehrstufige Aufgaben gleichermaßen.
- **Programmieren**: Repositorys bearbeiten, Builds und Tests ausführen, jede Änderung als Diff prüfen.
- **Recherche**: im Web suchen und Seiten, PDFs und Dokumente lesen.
- **Echter Browser**: Demi steuert Chrome. Schau zu und übernimm jederzeit.
- **Handy-Apps**: Apps auf iOS-Simulatoren und Android-Emulatoren installieren und testen.
- **Dateien und Daten**: Dokumente, Tabellen und Bilder, zurück als Anhänge.
- **Bilder, Video und Audio**: erstellt über die Modellanbieter, die du hinzufügst.
- **Kommentare und Fragen**: Wörter in einer Antwort markieren und danach fragen.
- **Teilen und suchen**: schreibgeschützte Links teilen und jede Nachricht durchsuchen.

### Überall, auf jedem Rechner

- **Jeder Browser**: Computer, Tablet, Handy oder Foldable, ohne Installation.
- **Immer aktiv**: Aufgaben laufen weiter, wenn du den Tab schließt, und benachrichtigen dich.
- **Ein Zuhause für Unterhaltungen**: am Laptop beginnen, am Handy weitermachen.
- **Private Cloud-Sandbox**: jeder Nutzer bekommt seine eigene Linux-Sandbox.
- **Deine eigenen Rechner**: macOS, Windows und Linux mit einem Befehl verbinden.
- **Viele Rechner, eine Aufgabe**: auf dem Mac bauen, unter Windows testen, mittendrin wechseln.
- **Dateien überall**: die Dateien jedes Rechners durchsuchen, ansehen und durchstöbern.
- **Portweiterleitung**: einen Port eines Rechners von einem anderen erreichen.
- **Anmeldung nach Wahl**: per QR-Code oder Passkey, in neun Sprachen.

### Agent

- **Multi-Agent**: ein Team von Agenten teilt eine große Aufgabe auf und arbeitet parallel.
- **Profile und Workflows**: spezialisierte Agenten, geteilt über dein Repository.
- **Zeitpläne**: eine Unterhaltung per Timer, per Cron oder bei einer Änderung wecken.
- **Unterhaltungen arbeiten zusammen**: eine Unterhaltung schreibt einer anderen oder liest sie.
- **Skills**: Demi neue Aufgaben beibringen.
- **Sichere Zugangsdaten**: Geheimnisse gehen über ein Formular, nie durch die Unterhaltung.

### Modelle und Teams

- **Deine Abos**: Claude, ChatGPT und Grok Build, mit Kontowechsel an ihren Limits.
- **Jede API**: Anthropic, OpenAI, Gemini, Coding-Pläne wie Kimi, GLM oder MiniMax und lokale Modelle.
- **Modell jederzeit wechseln**, auch mitten in einer Aufgabe.
- **Nutzung und Kosten**: die Limits jedes Abos und die Kosten der Arbeit sehen.
- **Mehrbenutzer**: jeder Nutzer hat eigene Sandbox, Geräte und Unterhaltungen.

## Ein Agent auf Basis einer Shell

Demi ist kein Wrapper um Claude Code oder Codex: Seine eigene Laufzeitumgebung
steuert jedes Modell. Wo die meisten Agenten dem Modell eine lange Liste von
Werkzeugen geben, gibt Demi ihm eines, eine Shell. Der Browser, andere Rechner
und Subagenten sind `demi`-Befehle darin:

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **Kombinierbar**: Befehle verketten sich mit Pipes, sodass ein Schritt die Arbeit mehrerer Tool-Aufrufe erledigt.
- **Schlanke Prompts**: Das Modell liest das `--help` eines Befehls nur, wenn es es braucht.
- **Überall dieselbe Shell**: eine bash-kompatible Shell, in Rust geschrieben, auf jedem Rechner.
- **Gleich für jedes Modell**: Ein Modellwechsel ändert nicht, was der Agent kann.
- **Für lange Aufgaben gebaut**: Befehle laufen im Hintergrund und melden sich zurück.

> [!NOTE]
> **Geplant**: eine OpenAPI für den Demi-Server, damit deine eigenen Programme
> seine Unterhaltungen, Agenten und Cloud-Sandboxes nutzen können.

## Architektur

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

- **Der Server betreibt den Agenten, Runner erledigen die Arbeit**: Jeder
  Runner verbindet sich nach außen und funktioniert daher hinter NAT und
  Firewalls.
- **Ein Runner überall**: Jede Funktion funktioniert auf jedem Rechner.
- **Sparsam mit Bandbreite**: Große Dateien bleiben, wo sie entstanden sind, bis du sie öffnest.
- **Nativ**: Rust für Server, Runner und Werkzeuge; Vue und TypeScript für die Web-App.

Lies die [Designdokumentation](docs/README.md), beginnend mit der
[Übersicht](docs/overview.md) (auf Englisch).

## Vergleich

Wie sich Demi von OpenHands, Orca, T3 Code, DeepSeek Harness, opencode, Pi und
herdr unterscheidet, Funktion für Funktion: [Demi im
Vergleich](docs/comparison.de.md).

## Dokumentation

- [Designdokumentation](docs/README.md): wie jeder Teil von Demi funktioniert
- [Mitwirken](CONTRIBUTING.md): Entwicklungsumgebung, Prüfungen und das
  Hinzufügen eines Anbieters
- [Sicherheit](SECURITY.md): Schwachstellen melden und wo Daten gespeichert
  werden

Diese Dokumente sind auf Englisch.

## Lizenz

Demi wird unter der [Apache License 2.0](LICENSE) veröffentlicht.
