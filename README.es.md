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
  <a href="#licencia"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0%20%2B%20Demi%20License-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.de.md">Deutsch</a> · <b>Español</b> · <a href="README.fr.md">Français</a> · <a href="README.pt-BR.md">Português</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="Demi en un navegador: un agente crea un diseño de dos columnas para una app de iPhone, con un comentario y una pregunta al margen sobre su respuesta y respuestas sugeridas debajo, la app en un iPhone Duo medio plegado en el panel lateral y el mismo chat en un teléfono." width="800">
</p>

Demi es un agente de IA basado en la web que alojas en tu propio servidor.
Ábrelo en cualquier navegador. Hazle una pregunta o encárgale un trabajo:
investigar, usar sitios web, trabajar con archivos, escribir código, todo en el
mismo chat.

Cada chat vive en tu servidor, en un solo lugar para todos tus dispositivos. Tus
computadoras solo ejecutan el trabajo: el agente trabaja en una sandbox en la
nube o en tus propias máquinas, y las tareas siguen en marcha cuando cierras la
pestaña.

<p align="center">
  <img src="docs/assets/concept.webp" alt="Desde una computadora o un teléfono llegas a un servidor de Demi, que guarda todos los chats y trabaja mediante runners en tu laptop, tu equipo de escritorio, tu HomeLab y tu VPS, o en sandboxes en la nube." width="800">
</p>

> [!NOTE]
> Demi está en una etapa temprana de desarrollo. Espera cambios incompatibles
> entre versiones.

## Inicio rápido

Necesitas lo siguiente:

- Un servidor Linux en el que tengas acceso root, con uno de estos sistemas
  ([distribuciones compatibles](docs/delivery/installation.md#distributions)):
  - Ubuntu 22.04 o posterior, Debian 12 o posterior
  - Fedora, RHEL 10, Rocky Linux, AlmaLinux
  - Arch Linux
- Un nombre de dominio servido por HTTPS delante del servidor ([configurar
  HTTPS](docs/delivery/installation.md#https)).

Ejecuta el comando en el servidor. Instala Beta, las versiones que Demi publica;
`--canary` instala Canary, que recibe primero cada compilación:

```sh
# Beta
curl -fsSL https://demi.sh/install.sh | sudo bash
# Canary
curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --canary
```

O dale este prompt a un agente de programación como Claude Code o Codex:

```text
Instala Demi en mi servidor siguiendo https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md
```

El instalador hace unas preguntas y configura Demi. Cuando termina:

1. Abre tu dominio de inmediato y crea la cuenta de administrador.
2. Ve a **Configuración › Proveedores** e inicia sesión con una suscripción o
   agrega una clave de API.
3. Inicia un chat. Se ejecuta en tu sandbox en la nube.
4. Para trabajar en tu propia computadora, elige **Agregar dispositivo** en el
   menú de host del chat y ejecuta en esa computadora el comando que muestra.

Consulta la [guía de instalación](docs/delivery/installation.md) para ver todas
las opciones, incluida la instalación sin preguntas, y las
[actualizaciones](docs/delivery/upgrades.md) para pasar a una versión nueva.

## Funciones principales

Las funciones de Demi se dividen en cuatro grupos.

### Chatear, trabajar y programar

- **Un chat para todo**: Haz una pregunta rápida o encarga un trabajo de varios
  pasos en el mismo lugar. Sin modos que cambiar.
- **Programación**: Edita repositorios, ejecuta compilaciones y pruebas, y
  revisa cada cambio como un diff.
- **Investigación**: Busca en la web y lee páginas, PDF y documentos.
- **Navegador real**: Demi controla Chrome para iniciar sesión, llenar
  formularios y descargar archivos. Míralo en el panel lateral y toma el control
  cuando quieras.
- **Apps para teléfono**: Demi controla simuladores de iOS y emuladores de
  Android para instalar, ejecutar y probar una app, y graba la pantalla. Míralos
  en el panel lateral y toma el control cuando quieras.
- **Archivos y datos**: Trabaja con documentos, hojas de cálculo e imágenes, y
  recibe los resultados como archivos adjuntos.
- **Imágenes, video y audio**: Crea y edita imágenes, y genera videos, voz,
  música y efectos de sonido con los proveedores de modelos que agregues.
- **Compartir**: Comparte un chat, o una sola respuesta, como un enlace de solo
  lectura.
- **Comentarios y preguntas**: Selecciona palabras de una respuesta para
  comentarlas o preguntar sobre ellas, sin interrumpir la tarea.
- **Búsqueda**: Encuentra cualquier mensaje en todos los chats, o en el que
  estás leyendo.

### En cualquier lugar, en cualquier máquina

- **Funciona en cualquier navegador**: Computadora, tableta o teléfono, sin
  instalar nada. Agrégalo a tu pantalla de inicio, y se adapta a teléfonos,
  tabletas y plegables.
- **Nueve idiomas**: Inglés, alemán, español, francés, portugués, chino
  simplificado, chino tradicional, japonés y coreano.
- **Siempre activo**: Las tareas siguen ejecutándose en el servidor después de
  cerrar la pestaña.
- **Notificaciones**: Recibe avisos en tu teléfono y tus computadoras cuando una
  tarea termina o te necesita, incluso con Demi cerrado.
- **Un solo lugar para todos los chats**: El servidor guarda los chats en vez de
  dispersarlos entre las máquinas en las que trabajó el agente. Empieza en tu
  laptop y sigue en tu teléfono, en vivo. Cambia de proveedor de IA y conserva
  tu historial.
- **Inicio de sesión con código QR y llave de acceso**: Escanea un código para
  iniciar sesión en tu teléfono, o usa Touch ID, Face ID o Windows Hello.
- **Sandbox privada en la nube**: Cada usuario tiene su propia sandbox de Linux
  en tu servidor. Se ejecuta con gVisor, no puede llegar a los servicios de tu
  servidor ni a tu red privada, y no ve las sandboxes de otros usuarios.
- **Usa tus propias computadoras**: Conecta máquinas macOS, Windows y Linux con
  un solo comando. No hace falta abrir puertos.
- **Tareas en varias máquinas**: Usa varias máquinas en un chat, por ejemplo
  para compilar en una Mac y probar en Windows.
- **Cambia de máquina a mitad de una tarea**: Mueve un chat entre la nube y tus
  computadoras sin empezar de nuevo.
- **Archivos en cada máquina**: Explora los archivos de la sandbox y de cada una
  de tus computadoras, previsualiza imágenes, videos, audio, PDF y Markdown,
  carga y descarga archivos, y busca nombres de archivo y texto.
- **Reenvío de puertos**: Llega a un puerto de una máquina desde otra.

### Agente

- **Varios agentes**: Demi divide una tarea grande entre un equipo de agentes
  que trabajan en paralelo y luego combina sus resultados.
- **Perfiles de agente**: Define agentes especializados, como uno de
  investigación o uno de revisión, cada uno con su propio modelo, nivel de
  razonamiento, instrucciones y comandos.
- **Flujos de trabajo**: Agrupa un conjunto de perfiles de agente con
  instrucciones para un tipo de trabajo. Guarda los flujos de trabajo en
  `.demi/workflows/` de un proyecto, elige uno en el cuadro de mensaje y
  compártelos con tu equipo a través del repositorio.
- **Programaciones**: Activa un chat una vez, en un ciclo que se repite, a
  intervalos, con cron, cuando cambia la salida de un comando o cuando se llama
  a una URL.
- **Los chats trabajan juntos**: Un chat envía mensajes a otro y lee los
  historiales de otros chats.
- **Habilidades**: Instala habilidades para enseñarle a Demi tareas nuevas.
- **Credenciales seguras**: Demi pide claves de API y contraseñas mediante un
  formulario seguro, y nunca aparecen en el chat.

### Modelos y equipos

- **Usa tus suscripciones**: Inicia sesión con cuentas de Claude, ChatGPT o Grok
  Build. Agrega varias cuentas por suscripción, y Demi rota entre ellas a medida
  que se agotan sus límites de uso.
- **Planes de programación**: Usa la clave de un plan de programación, como
  Kimi, GLM o MiniMax.
- **Claves de API**: Anthropic, OpenAI, Gemini o cualquier endpoint compatible.
- **Modelos locales**: Permite que tu servidor use un modelo que se ejecuta en
  tu propia computadora, como Ollama, mediante el reenvío de puertos.
- **Cambia de modelo cuando quieras**: Cambia el modelo o el nivel de
  razonamiento, incluso a mitad de una tarea.
- **Uso y costo**: Mira cuánto se usó de los límites de cada suscripción y
  cuándo se restablecen, y cuánto costó el trabajo en la última hora, el último
  día o en total.
- **Varios usuarios**: Cada usuario tiene su propia sandbox, sus dispositivos y
  sus chats, con las suscripciones del administrador o las suyas.
- **Herramientas de administración**: Agrega usuarios, administra roles,
  restablece contraseñas y envía códigos de inicio de sesión para el teléfono.
- **Instalación y actualizaciones en una línea**: Configura un servidor con un
  comando y actualízalo con otro.

## Un agente construido sobre una shell

Demi es a la vez la app y el agente.

No es un envoltorio de Claude Code ni de Codex. Su propio runtime controla todos
los modelos, y una suscripción a Claude o a ChatGPT solo aporta el modelo.

La mayoría de los agentes le dan al modelo una larga lista de herramientas. Demi
le da una: una shell. Todo lo demás, desde el navegador hasta otras máquinas y
los subagentes, es un comando `demi` dentro de ella:

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

El diseño tiene estas ventajas:

- **Componible**: Los comandos se encadenan con tuberías y `&&`, así que el
  agente hace en un paso lo que normalmente requiere varias llamadas a
  herramientas.
- **Prompts ligeros**: El modelo ve una línea por grupo de comandos y lee
  `--help` cuando necesita más. Las capacidades nuevas no inflan cada solicitud.
- **La misma shell en todas partes**: Cada máquina ejecuta la misma shell
  compatible con bash, escrita en Rust
  ([brush](https://github.com/reubeno/brush)), así que un script que funciona en
  la nube también funciona en macOS y Windows. Las herramientas que modifican
  archivos son las de GNU en todas las máquinas
  ([uutils](https://github.com/uutils/coreutils)), así que sus cambios quedan
  registrados; cualquier otro programa, como `tar` o `python`, es el del sistema
  y sus cambios no se registran. En Windows, las demás utilidades son las de Git
  for Windows, que la configuración de Demi instala si falta.
- **Igual para todos los modelos**: Todos los proveedores ven la misma
  herramienta única, así que cambiar de modelo no cambia lo que puede hacer el
  agente.
- **Hecho para trabajos largos**: Los comandos siguen ejecutándose en segundo
  plano e informan cuando terminan, así que los servidores de desarrollo y las
  suites de pruebas nunca bloquean el chat.

> [!NOTE]
> **Planeado**: una OpenAPI para el servidor de Demi, para que tus propios
> programas usen sus chats, agentes y sandboxes en la nube para los agentes que
> necesites.

## Arquitectura

El siguiente diagrama muestra cómo se conectan las partes:

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

El diseño tiene estas propiedades:

- **El servidor ejecuta el agente y los runners hacen el trabajo**: Los chats,
  los agentes y la configuración viven en el servidor. Un pequeño runner en cada
  máquina solo se encarga de archivos, comandos y el navegador. Se conecta hacia
  afuera con el servidor, así que funciona detrás de NAT y firewalls.
- **Un solo runner en todas partes**: La sandbox en la nube y tus computadoras
  ejecutan el mismo runner, así que todas las funciones están disponibles en
  todas las máquinas.
- **Poco ancho de banda**: Las salidas y los archivos grandes se quedan en la
  máquina que los generó. El navegador los carga solo cuando los abres.
- **Nativo**: El servidor, el runner y las herramientas del agente están
  escritos en Rust. La app web está hecha con Vue y TypeScript.

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

La mayor parte de Demi se publica bajo la [Apache License 2.0](LICENSE). Puedes
usar estas partes para cualquier fin:

- El runtime del agente.
- Los proveedores de modelos y la biblioteca que los configura a partir de tu
  configuración.
- El runner que instalas en tus máquinas y la biblioteca que lo controla.
- Los comandos.
- Los plugins, su SDK y el host que los ejecuta.
- La biblioteca de componentes de interfaz y su galería.
- Los protocolos.
- La documentación.

El servidor y la app web (`crates/backend*`, `crates/server`,
`crates/machine-manager` y `packages/web`) se publican bajo la [Demi
License](LICENSE-DEMI), que es la Apache License 2.0 con condiciones
adicionales:

- **Uso libre**: Úsalos, modifícalos y ejecútalos, incluso en tu empresa para tu
  propio equipo.
- **Soporte libre**: Cobra por implementar, integrar o dar soporte a
  instalaciones de Demi que otros ejecutan por su cuenta.
- **Sin reventa como servicio**: No los ofrezcas a otros a cambio de un pago
  como servicio alojado o multiinquilino, ni como un producto que sea
  esencialmente Demi.
- **Conserva la marca**: No quites ni cambies el nombre, el logotipo ni los
  avisos de copyright de Demi en la app web.

Para otros usos, escribe a [support@demi.sh](mailto:support@demi.sh).
