<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>Tu agente, en todas tus máquinas.<br>
  Un agente de IA basado en la web que alojas en tu propio servidor.</strong>
</p>

<p align="center">
  <a href="#licencia"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.de.md">Deutsch</a> · <b>Español</b> · <a href="README.fr.md">Français</a> · <a href="README.pt-BR.md">Português</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="Demi en un navegador: un agente crea un diseño de dos columnas para una app de iPhone, con un comentario y una pregunta al margen sobre su respuesta y respuestas sugeridas debajo, la app en un iPhone Duo medio plegado en el panel lateral y el mismo chat en un teléfono." width="800">
</p>

Demi es un agente de IA web que alojas en tu propio servidor. Ábrelo en
cualquier navegador y encárgale lo que sea: investigación, sitios web, archivos
o código. Tus conversaciones se quedan en tu servidor, y el trabajo se ejecuta
en un sandbox en la nube o en tus propias máquinas, incluso después de cerrar
la pestaña.

> [!NOTE]
> Demi está en una fase temprana de desarrollo. Espera cambios incompatibles
> entre versiones.

## Inicio rápido

Despliega Demi en unos clics:

<p>
  <a href="docs/delivery/railway.md"><img alt="Deploy on Railway" src="https://railway.com/button.svg" height="36"></a>
  <a href="https://render.com/deploy?repo=https://github.com/demi-sh/demi"><img alt="Deploy on Render" src="docs/assets/deploy/deploy-on-render.svg" height="36"></a>
  <a href="docs/delivery/fly.md"><img alt="Deploy on Fly.io" src="docs/assets/deploy/deploy-on-fly.svg" height="36"></a>
  <a href="docs/delivery/docker-compose.md"><img alt="Run with Docker Compose" src="docs/assets/deploy/run-with-docker-compose.svg" height="36"></a>
</p>

O instálalo en un servidor Linux propio, con un nombre de dominio:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

O pide a un agente de programación, como Claude Code o Codex, que lo instale:

```text
Instala Demi en mi servidor siguiendo https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md
```

Después abre Demi, crea la cuenta de administrador y añade un modelo en
**Configuración › Proveedores**. La [guía de instalación](docs/delivery/installation.md)
cubre los sistemas compatibles, HTTPS, el canal Canary y lo que cuesta cada
opción.

<p align="center">
  <img src="docs/assets/concept.webp" alt="Desde una computadora o un teléfono llegas a un servidor de Demi, que guarda todos los chats y trabaja mediante runners en tu laptop, tu equipo de escritorio, tu HomeLab y tu VPS, o en sandboxes en la nube." width="800">
</p>

## Funciones principales

### Chatear, trabajar y programar

- **Una conversación para todo**: preguntas rápidas y tareas de varios pasos por igual.
- **Programación**: edita repositorios, ejecuta compilaciones y pruebas, revisa cada cambio como diff.
- **Investigación**: busca en la web y lee páginas, PDF y documentos.
- **Navegador real**: Demi maneja Chrome. Obsérvalo y toma el control cuando quieras.
- **Apps móviles**: instala y prueba apps en simuladores de iOS y emuladores de Android.
- **Archivos y datos**: documentos, hojas de cálculo e imágenes, devueltos como adjuntos.
- **Imágenes, vídeo y audio**: creados con los proveedores de modelos que añadas.
- **Comentarios y preguntas**: selecciona palabras de una respuesta para preguntar sobre ellas.
- **Compartir y buscar**: comparte un enlace de solo lectura y busca en cada mensaje.

### En cualquier lugar, en cualquier máquina

- **Cualquier navegador**: ordenador, tableta, teléfono o plegable, sin instalar nada.
- **Siempre activo**: las tareas siguen al cerrar la pestaña y te avisan.
- **Un solo hogar para las conversaciones**: empieza en el portátil y sigue en el teléfono.
- **Sandbox privado en la nube**: cada usuario tiene su propio sandbox de Linux.
- **Tus propios ordenadores**: conecta macOS, Windows y Linux con un comando.
- **Muchas máquinas, una tarea**: compila en un Mac, prueba en Windows, cambia a mitad de tarea.
- **Archivos en todas partes**: explora, previsualiza y busca los archivos de cada máquina.
- **Reenvío de puertos**: accede a un puerto de una máquina desde otra.
- **Inicio de sesión a tu manera**: código QR o passkey, en nueve idiomas.

### Agente

- **Multiagente**: un equipo de agentes divide una tarea grande y trabaja en paralelo.
- **Perfiles y flujos de trabajo**: agentes especializados, compartidos a través de tu repositorio.
- **Programaciones**: despierta una conversación con un temporizador, con cron o ante un cambio.
- **Las conversaciones colaboran**: una conversación escribe a otra o la lee.
- **Skills**: enseña nuevas tareas a Demi.
- **Credenciales seguras**: los secretos pasan por un formulario, nunca por la conversación.

### Modelos y equipos

- **Tus suscripciones**: Claude, ChatGPT y Grok Build, rotando cuentas al llegar a sus límites.
- **Cualquier API**: Anthropic, OpenAI, Gemini, planes de programación como Kimi, GLM o MiniMax, y modelos locales.
- **Cambia de modelo cuando quieras**, incluso a mitad de una tarea.
- **Uso y coste**: consulta los límites de cada suscripción y lo que costó el trabajo.
- **Multiusuario**: cada usuario tiene su propio sandbox, dispositivos y conversaciones.

## Un agente construido sobre una shell

Demi no envuelve Claude Code ni Codex: su propio runtime controla cada modelo.
Donde la mayoría de los agentes dan al modelo una larga lista de herramientas,
Demi le da una, una shell. El navegador, otras máquinas y los subagentes son
comandos `demi` dentro de ella:

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi device shell --device windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **Componible**: los comandos se encadenan con tuberías, así que un paso hace el trabajo de varias llamadas a herramientas.
- **Prompts ligeros**: el modelo lee el `--help` de un comando solo cuando lo necesita.
- **La misma shell en todas partes**: una shell compatible con bash, escrita en Rust, en cada máquina.
- **Igual para cada modelo**: cambiar de modelo no cambia lo que puede hacer el agente.
- **Hecho para tareas largas**: los comandos se ejecutan en segundo plano e informan al terminar.

> [!NOTE]
> **Previsto**: una OpenAPI para el servidor de Demi, para que tus propios
> programas usen sus conversaciones, agentes y sandboxes en la nube.

## Arquitectura

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

- **El servidor ejecuta el agente, los runners hacen el trabajo**: cada runner
  se conecta hacia fuera, así que funciona detrás de NAT y cortafuegos.
- **Un runner en todas partes**: cada función funciona en cada máquina.
- **Ligero en ancho de banda**: los archivos grandes se quedan donde se crearon hasta que los abres.
- **Nativo**: Rust para el servidor, el runner y las herramientas; Vue y TypeScript para la aplicación web.

Lee la [documentación de diseño](docs/README.md), empezando por la [descripción
general](docs/overview.md) (en inglés).

## Comparación

En qué se diferencia Demi de OpenHands, Orca, T3 Code, DeepSeek Harness,
opencode, Pi y herdr, función por función:
[Demi comparado](docs/comparison.es.md).

## Documentación

- [Documentación de diseño](docs/README.md): cómo funciona cada parte de Demi
- [Contribuir](CONTRIBUTING.md): entorno de desarrollo, verificaciones y cómo
  agregar un proveedor
- [Seguridad](SECURITY.md): cómo informar vulnerabilidades y dónde se guardan
  los datos

Estos documentos están en inglés.

## Licencia

Demi se publica bajo la [Apache License 2.0](LICENSE).
