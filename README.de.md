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
  <a href="#lizenz"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0%20%2B%20Demi%20License-blue"></a>
  <a href="https://github.com/wspl/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/wspl/demi"></a>
  <a href="https://github.com/wspl/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/wspl/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <b>Deutsch</b> · <a href="README.es.md">Español (Latinoamérica)</a> · <a href="README.fr.md">Français</a> · <a href="README.pt-BR.md">Português (Brasil)</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="Demi im Browser: Ein Agent baut ein zweispaltiges Layout für eine iPhone-App, mit einem Kommentar und einer Nebenfrage zu seiner Antwort und Antwortvorschlägen darunter, die App auf einem halb gefalteten iPhone Duo im Seitenpanel und derselbe Chat auf einem Telefon." width="800">
</p>

Demi ist ein webbasierter KI-Agent, den du auf deinem eigenen Server hostest.
Öffne ihn in jedem Browser. Stell ihm eine Frage oder übergib ihm eine Aufgabe:
Recherche, Websites bedienen, mit Dateien arbeiten, Code schreiben, alles im
selben Chat.

Jeder Chat liegt auf deinem Server, an einem Ort für all deine Geräte. Deine
Computer führen nur die Arbeit aus: Der Agent arbeitet in einer Cloud-Sandbox
oder auf deinen eigenen Rechnern, und Aufgaben laufen weiter, wenn du den Tab
schließt.

<p align="center">
  <img src="docs/assets/concept.webp" alt="Von einem Computer oder einem Telefon erreichst du einen Demi-Server, der jeden Chat speichert und über Runner auf deinem Laptop, Desktop, HomeLab und VPS oder in Cloud-Sandboxes arbeitet." width="800">
</p>

> [!NOTE]
> Demi befindet sich in einer frühen Entwicklungsphase. Rechne mit inkompatiblen
> Änderungen zwischen Releases.

## Schnellstart

Du brauchst Folgendes:

- Einen Linux-Server, auf dem du Root-Rechte hast, mit einem dieser Systeme
  ([unterstützte Distributionen](docs/delivery/installation.md#distributions)):
  - Ubuntu 22.04 oder neuer, Debian 12 oder neuer
  - Fedora, RHEL 10, Rocky Linux, AlmaLinux
  - Arch Linux
- Einen Domainnamen, der vor dem Server über HTTPS ausgeliefert wird ([HTTPS
  einrichten](docs/delivery/installation.md#https)).

Führe den Befehl auf dem Server aus. Er installiert Beta, die Releases, die Demi
veröffentlicht; `--canary` installiert Canary, das jeden Build als Erstes
erhält:

```sh
# Beta
curl -fsSL https://demi.sh/install.sh | sudo bash
# Canary
curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --canary
```

Oder gib einem Coding-Agenten wie Claude Code oder Codex diesen Prompt:

```text
Installiere Demi auf meinem Server nach dieser Anleitung: https://raw.githubusercontent.com/wspl/demi/main/docs/delivery/install-with-an-agent.md
```

Das Installationsprogramm stellt ein paar Fragen und richtet Demi ein. Danach:

1. Öffne sofort deine Domain und erstelle das Administratorkonto.
2. Gehe zu **Einstellungen › Anbieter** und melde dich mit einem Abo an oder
   füge einen API-Schlüssel hinzu.
3. Beginne einen Chat. Er läuft in deiner Cloud-Sandbox.
4. Um auf deinem eigenen Computer zu arbeiten, wähle im Host-Menü des Chats
   **Gerät hinzufügen** und führe den angezeigten Befehl auf diesem Computer
   aus.

Alle Optionen, auch die Installation ohne Fragen, stehen in der
[Installationsanleitung](docs/delivery/installation.md), der Wechsel auf ein
neues Release unter [Upgrades](docs/delivery/upgrades.md).

## Wichtige Funktionen

Die Funktionen von Demi gliedern sich in vier Gruppen.

### Chatten, arbeiten und programmieren

- **Ein Chat für alles**: Stell eine kurze Frage oder übergib eine mehrstufige
  Aufgabe am selben Ort. Keine Modi zum Umschalten.
- **Programmieren**: Bearbeite Repositorys, führe Builds und Tests aus und prüfe
  jede Änderung als Diff.
- **Recherche**: Durchsuche das Web und lies Seiten, PDFs und Dokumente.
- **Echter Browser**: Demi steuert Chrome, um sich anzumelden, Formulare
  auszufüllen und Dateien herunterzuladen. Sieh im Seitenpanel zu und übernimm
  jederzeit.
- **Telefon-Apps**: Demi steuert iOS-Simulatoren und Android-Emulatoren, um eine
  App zu installieren, auszuführen und zu testen, und nimmt den Bildschirm auf.
  Sieh im Seitenpanel zu und übernimm jederzeit.
- **Dateien und Daten**: Arbeite mit Dokumenten, Tabellen und Bildern und
  erhalte die Ergebnisse als Anhänge zurück.
- **Bilder, Video und Audio**: Erstelle und bearbeite Bilder und erzeuge Videos,
  Sprache, Musik und Soundeffekte über die Modellanbieter, die du hinzufügst.
- **Teilen**: Teile einen Chat oder eine einzelne Antwort als schreibgeschützten
  Link.
- **Kommentare und Fragen**: Markiere Wörter in einer Antwort, um sie zu
  kommentieren oder danach zu fragen, ohne die Aufgabe zu unterbrechen.
- **Suche**: Finde jede Nachricht in allen Chats oder in dem, den du gerade
  liest.

### Überall, auf jedem Rechner

- **Läuft in jedem Browser**: Desktop, Tablet oder Telefon, ohne etwas zu
  installieren. Füge es zum Home-Bildschirm hinzu, und es passt sich an
  Telefone, Tablets und Faltgeräte an.
- **Neun Sprachen**: Englisch, Deutsch, Spanisch, Französisch, Portugiesisch,
  vereinfachtes Chinesisch, traditionelles Chinesisch, Japanisch und Koreanisch.
- **Immer an**: Aufgaben laufen auf dem Server weiter, nachdem du den Tab
  geschlossen hast.
- **Benachrichtigungen**: Werde auf deinem Telefon und deinen Computern
  benachrichtigt, wenn eine Aufgabe fertig ist oder dich braucht, auch wenn Demi
  geschlossen ist.
- **Ein Ort für jeden Chat**: Der Server speichert die Chats, statt sie über die
  Rechner zu verstreuen, auf denen der Agent gearbeitet hat. Beginne auf dem
  Laptop und mach live auf dem Telefon weiter. Wechsle den KI-Anbieter und
  behalte deinen Verlauf.
- **Anmeldung per QR-Code und Passkey**: Scanne einen Code, um dich auf dem
  Telefon anzumelden, oder verwende Touch ID, Face ID oder Windows Hello.
- **Private Cloud-Sandbox**: Jeder Benutzer erhält eine eigene Linux-Sandbox auf
  deinem Server. Sie läuft unter gVisor, erreicht weder die Dienste deines
  Servers noch dein privates Netzwerk und sieht die Sandboxes anderer Benutzer
  nicht.
- **Eigene Computer anbinden**: Verbinde macOS-, Windows- und Linux-Rechner mit
  einem einzigen Befehl. Keine offenen Ports nötig.
- **Aufgaben über mehrere Rechner**: Verwende mehrere Rechner in einem Chat,
  etwa um auf einem Mac zu bauen und unter Windows zu testen.
- **Rechner mitten in der Aufgabe wechseln**: Verschiebe einen Chat zwischen der
  Cloud und deinen Computern, ohne neu anzufangen.
- **Dateien auf jedem Rechner**: Durchsuche die Dateien der Sandbox und jedes
  deiner Computer, zeige Bilder, Videos, Audio, PDFs und Markdown in der
  Vorschau, lade hoch und herunter und durchsuche Dateinamen und Text.
- **Portweiterleitung**: Erreiche einen Port eines Rechners von einem anderen
  aus.

### Agent

- **Mehrere Agenten**: Demi teilt eine große Aufgabe auf ein Team von Agenten
  auf, die parallel arbeiten, und führt ihre Ergebnisse zusammen.
- **Agentenprofile**: Lege spezialisierte Agenten an, etwa für Recherche oder
  Review, jeder mit eigenem Modell, Denkaufwand, eigenen Anweisungen und
  Befehlen.
- **Workflows**: Bündle eine Reihe von Agentenprofilen mit Anweisungen für eine
  Art von Arbeit. Lege Workflows im Ordner `.demi/workflows/` eines Projekts ab,
  wähle einen im Nachrichtenfeld aus und teile sie über das Repository mit
  deinem Team.
- **Zeitpläne**: Wecke einen Chat einmal, in einem wiederkehrenden Rhythmus, in
  Intervallen, per Cron, wenn sich die Ausgabe eines Befehls ändert oder wenn
  eine URL aufgerufen wird.
- **Chats arbeiten zusammen**: Ein Chat sendet Nachrichten an einen anderen und
  liest die Verläufe anderer Chats.
- **Skills**: Installiere Skills, um Demi neue Aufgaben beizubringen.
- **Sichere Zugangsdaten**: Demi fragt API-Schlüssel und Passwörter über ein
  sicheres Formular ab, und sie erscheinen nie im Chat.

### Modelle und Teams

- **Nutze deine Abos**: Melde dich mit Konten von Claude, ChatGPT oder Grok
  Build an. Füge mehrere Konten pro Abo hinzu, und Demi wechselt zwischen ihnen,
  wenn ihre Nutzungslimits ausgeschöpft sind.
- **Coding-Pläne**: Verwende den Schlüssel eines Coding-Plans, etwa von Kimi,
  GLM oder MiniMax.
- **API-Schlüssel**: Anthropic, OpenAI, Gemini oder jeder kompatible Endpunkt.
- **Lokale Modelle**: Lass deinen Server über Portweiterleitung ein Modell
  nutzen, das auf deinem eigenen Computer läuft, etwa mit Ollama.
- **Modelle jederzeit wechseln**: Ändere das Modell oder den Denkaufwand, auch
  mitten in einer Aufgabe.
- **Nutzung und Kosten**: Sieh, wie viel der Limits jedes Abos verbraucht ist
  und wann sie zurückgesetzt werden, und was die Arbeit in der letzten Stunde,
  am letzten Tag oder insgesamt gekostet hat.
- **Mehrere Benutzer**: Jeder Benutzer hat eigene Sandbox, Geräte und Chats, mit
  den Abos des Admins oder eigenen.
- **Admin-Werkzeuge**: Füge Benutzer hinzu, verwalte Rollen, setze Passwörter
  zurück und sende Anmeldecodes fürs Telefon.
- **Installation und Upgrades in einer Zeile**: Richte einen Server mit einem
  Befehl ein und aktualisiere ihn mit einem anderen.

## Ein Agent auf Basis einer Shell

Demi ist sowohl die App als auch der Agent.

Demi ist keine Hülle um Claude Code oder Codex. Seine eigene Laufzeitumgebung
steuert jedes Modell, und ein Abo von Claude oder ChatGPT liefert nur das
Modell.

Die meisten Agenten geben dem Modell eine lange Liste von Werkzeugen. Demi gibt
ihm eines: eine Shell. Alles andere, vom Browser über andere Rechner bis zu
Subagenten, ist ein `demi`-Befehl darin:

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

Das Design hat diese Vorteile:

- **Kombinierbar**: Befehle lassen sich mit Pipes und `&&` verketten, sodass der
  Agent in einem Schritt erledigt, wofür sonst mehrere Werkzeugaufrufe nötig
  sind.
- **Schlanke Prompts**: Das Modell sieht eine Zeile pro Befehlsgruppe und liest
  `--help`, wenn es mehr braucht. Neue Fähigkeiten blähen nicht jede Anfrage
  auf.
- **Überall dieselbe Shell**: Jeder Rechner führt dieselbe bash-kompatible Shell
  aus, geschrieben in Rust ([brush](https://github.com/reubeno/brush)), sodass
  ein Skript, das in der Cloud funktioniert, auch unter macOS und Windows
  funktioniert. Die Werkzeuge, die Dateien ändern, sind auf jedem Rechner die
  von GNU ([uutils](https://github.com/uutils/coreutils)), daher werden ihre
  Änderungen aufgezeichnet; jedes andere Programm, etwa `tar` oder `python`, ist
  das des Systems, und seine Änderungen werden nicht aufgezeichnet. Unter
  Windows stammen die übrigen Werkzeuge aus Git for Windows, das die Einrichtung
  von Demi installiert, wenn es fehlt.
- **Gleich für jedes Modell**: Jeder Anbieter sieht dasselbe eine Werkzeug,
  sodass ein Modellwechsel nicht ändert, was der Agent kann.
- **Für lange Aufgaben gebaut**: Befehle laufen im Hintergrund weiter und melden
  sich, wenn sie fertig sind, sodass Entwicklungsserver und Testsuiten den Chat
  nie blockieren.

> [!NOTE]
> **Geplant**: eine OpenAPI für den Demi-Server, damit deine eigenen Programme
> seine Chats, Agenten und Cloud-Sandboxes für die Agenten nutzen können, die du
> brauchst.

## Architektur

Das folgende Diagramm zeigt, wie die Teile zusammenhängen:

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

Das Design hat diese Eigenschaften:

- **Der Server führt den Agenten aus, Runner erledigen die Arbeit**: Chats,
  Agenten und Einstellungen liegen auf dem Server. Ein kleiner Runner auf jedem
  Rechner kümmert sich nur um Dateien, Befehle und den Browser. Er verbindet
  sich nach außen zum Server und funktioniert daher hinter NAT und Firewalls.
- **Überall ein Runner**: Die Cloud-Sandbox und deine Computer führen denselben
  Runner aus, sodass jede Funktion auf jedem Rechner funktioniert.
- **Sparsam mit Bandbreite**: Große Ausgaben und Dateien bleiben auf dem
  Rechner, der sie erzeugt hat. Der Browser lädt sie erst, wenn du sie öffnest.
- **Nativ**: Der Server, der Runner und die Werkzeuge des Agenten sind in Rust
  geschrieben. Die Web-App ist mit Vue und TypeScript gebaut.

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

Der größte Teil von Demi wird unter der [Apache License 2.0](LICENSE)
veröffentlicht. Diese Teile darfst du für alles verwenden:

- Die Agent-Laufzeitumgebung.
- Die Modellanbieter und die Bibliothek, die sie aus deiner Konfiguration
  einrichtet.
- Den Runner, den du auf deinen Rechnern installierst, und die Bibliothek, die
  ihn steuert.
- Die Befehle.
- Die Plug-ins, ihr SDK und den Host, der sie ausführt.
- Die UI-Komponentenbibliothek und ihre Galerie.
- Die Protokolle.
- Die Dokumentation.

Der Server und die Web-App (`crates/backend*`, `crates/server`,
`crates/machine-manager` und `packages/web`) werden unter der [Demi
License](LICENSE-DEMI) veröffentlicht, der Apache License 2.0 mit zusätzlichen
Bedingungen:

- **Frei nutzbar**: Nutze, verändere und betreibe sie, auch in deinem
  Unternehmen für dein eigenes Team.
- **Frei unterstützbar**: Berechne Geld für das Bereitstellen, Integrieren oder
  Unterstützen von Demi, das andere selbst betreiben.
- **Kein Weiterverkauf als Dienst**: Biete sie anderen nicht gegen Entgelt als
  gehosteten oder mandantenfähigen Dienst an oder als Produkt, das im
  Wesentlichen Demi ist.
- **Marke beibehalten**: Entferne oder ändere den Namen, das Logo und die
  Urheberrechtshinweise von Demi in der Web-App nicht.

Für andere Nutzungen wende dich an [support@demi.sh](mailto:support@demi.sh).
