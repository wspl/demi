# Web preview

The agent started a development server on its Host, `http://localhost:3000`,
checked the page in the conversation browser, and wants the user to see it.
The [live view](live-view.md) shows the conversation browser as video: it
lags, its text is not as sharp as the user's own screen, and the user cannot
scroll, select and copy as they would in their browser. A web preview renders
the same page **in the user's own browser**, while every request the page
makes still leaves from the Host: the site sees the Host's network, DNS,
certificates and cookies, so `localhost:3000` of the Host opens as it does
there.

```text
the user's browser
  Demi page (the relay)
    └─ iframe https://k3f9a2ab--p4q8h2m6c1v9t7s2.demi-preview.dev/dashboard
         │ every request the browser makes for it
         ▼
       forwarder (that origin's service worker) ── MessagePort ──▶ relay
                                                                  │ the `preview` user stream:
                                                                  │ direct channel or relay
                                                                  ▼
                                              preview engine on the Host ──▶ http://localhost:3000
```

The preview was built and measured in a spike
(`/Users/zan/Spikes/sw-iframe-proxy`, 2026-10-07): 192 behavior cases agree
with the same page loaded directly, the hot reload of six frameworks keeps
its state, and 40 of 50 public sites look the same as in Chrome, the rest
differing for the reasons in [Known differences](#known-differences). This
document is the design as Demi builds it; the spike's code is copied into the
crates and packages [Parts](#parts) names.

## What the user sees

**Two kinds of tab in one strip.** The work panel's browser tabs are of two
kinds, told apart only by the icon before the title, as a browser shows a
page's icon:

| Tab | What it is | Who operates it | Icon |
| --- | --- | --- | --- |
| A tab of the agent's browser | The [live view](live-view.md) of a tab of the conversation browser | The agent and the user | One fixed icon for the agent's browser |
| A tab of your browser | A web preview, rendered by the user's browser | The user only; the agent cannot see it | The page's own icon, as in any browser |

There is no group, color bar or badge besides. The icons are the gallery's.

**Opening a page in the other browser.** Each kind offers the other, in the
tab's menu:

- on a tab of the agent's browser: **Open in Your Browser**;
- on a tab of your browser: **Open in Agent's Browser**.

Either opens a new tab of the other kind beside the tab and selects it; the
tab it came from stays as it is. The new tab takes the address, the page's
state ([Page state](#page-state)) and the size mode, Web or Mobile. There is
no switch that turns one tab into the other: the agent's tab would vanish
from the user's sight while the agent still holds it, and whether a tab that
looks the same can be operated by the agent would become a state nobody can
see.

**The strip's +** opens a tab of your browser, with an empty address bar, as
a browser's new tab does: the user browses natively, and hands a page to the
agent with Open in Agent's Browser.

**The agent hands a page over.** `demi browser present <tab>` says that the
user should see a tab of the agent's browser ([Presenting a
page](#presenting-a-page)). The command's block in the transcript shows a card
with the page's title, its address and **Open**: Open opens the page in a tab
of your browser, with its state, selects it, and opens the work panel when it
is closed. The agent never changes what the user is looking at by itself.

**Opening on a stopped Cloud** wakes it, as the agent's first `open` does,
and the tab says what it waits for as a tab of the agent's browser does
before it has its page ([A browser tab in the panel](live-view.md#a-browser-tab-in-the-panel)).

**The size menu** of a tab of your browser offers Web and Mobile; Custom is a
size only the agent sets, and does not appear. Choosing Mobile loads the page
again, for the reason [Modes](live-view.md#modes) gives
([Mobile](#mobile)).

**Where the preview cannot run.** The preview needs a page opened in a secure
context, HTTPS or `localhost`, and Chrome: a page opened at
`http://192.168.1.20` cannot register the service worker the preview needs.
There, the + and Open in Your Browser are unavailable and say why, and a
card's Open says so.

## Parts

| Part | Where it runs | Owns | Built from |
| --- | --- | --- | --- |
| Preview domain service | Cloudflare, one deployment for every Demi | Namespaces; `frame-ancestors` per namespace; the versioned boot page, client script and forwarder | `services/preview-domain` |
| Forwarder | Each preview origin's service worker, one script for all | Handing every request of its origin's documents to the relay, and the answers back | `services/preview-domain` |
| Client script | Each preview document's first script, from the preview domain | Asking the relay for the forwarder's channel | `services/preview-domain` |
| Relay | The Demi page, in `@demicodes/plugin-browser` | Binding each channel to its preview origin; keeping cross-origin navigations' requests; choosing the Host and the path to it; delivering the runtime | `@demicodes/plugin-browser` |
| Transport | The `preview` [user stream](../execution/native-runtime.md#user-streams) | Requests, responses and WebSocket messages, both ways, [framed](#the-stream) | `plugin-browser`'s stream declaration |
| Preview engine | `demi-browser` on the Host | Mapping preview and real addresses; the cookie jar; SameSite, CORS, CORP and local network rules; requests upstream with a browser's fingerprint; rewriting | `command-package-browser-preview` |
| Rewriter | The engine, and the runtime as WebAssembly | One rewriting of addresses, JavaScript, CSS and HTML | `preview-rewrite`, `preview-rewrite-wasm` |
| Runtime | Each preview document, loaded from the web app's build | Showing the page its real address and origin; doing what the browser cannot do through the forwarder | `@demicodes/preview-runtime` |

The engine runs in `demi-browser`, the conversation browser's resident program,
rather than a service of its own: [Page state](#page-state) moves cookies and
storage between the conversation's Chrome and the engine's cookie jar, and
both are in one process there. Responsibilities and dependencies of each crate
and package are in [Crates and packages](../architecture/crates-and-packages.md).

For example, the user opens `http://localhost:5173/editor` on Host H:

1. The page asks H's engine, through the browser plugin's page methods, for
   the preview origin L of the address's top-level environment
   ([Addresses and labels](#addresses-and-labels)), and opens an iframe on
   `https://L/__demi/v1/boot.html#to=/editor`.
2. The boot page registers the forwarder, gets a channel through the client
   script, and replaces itself with `/editor`.
3. The forwarder hands `GET https://L/editor` to the relay. The relay knows the
   channel belongs to L and to which tab, and sends the request on the tab's
   `preview` stream.
4. The engine maps `L/editor` back to `http://localhost:5173/editor`, requests
   it from H, rewrites the answer and sends it back the same way. The page's
   CSS, images and modules repeat steps 3 and 4.

## Addresses and labels

```text
https://<namespace>--<label>.demi-preview.dev/<path>?<query>#<fragment>
```

The path, query and fragment are the site's own, so relative and root-relative
addresses need no rewriting, and a front-end router, `<base>` and the relative
imports between modules work as they are.

- **Namespace**: 8 characters, one per Demi deployment
  ([The preview domain service](#the-preview-domain-service)).
- **Label**: 16 characters, 80 bits, naming one document environment: the
  logical origin, such as `http://localhost:5173`; the logical top-level site
  of the document's frame tree; and whether an ancestor of the document's
  frame, the logical top included, is cross-site to it. The label is the
  first 80 bits of the SHA-256 of (namespace, Host, logical origin, top-level
  site, cross-site ancestor), in lowercase base32hex (`0`–`9`, `a`–`v`). The
  algorithm never changes between releases, or what the browser stored under
  a label would be lost. The algorithm is public:
  the engine computes it when it rewrites, and the runtime when it maps
  `img.src = …` synchronously, with no key and no table. One environment
  always gets one label, so what the browser stores for it is there the next
  time.

The label is not a secret. Isolation comes from a label naming one
environment only: the relay accepts a mapping only after computing it again,
and the engine serves a label only its origin's content, so nobody can make
another site's content appear under a label, run code in its origin or read
its storage. Two things follow from putting the environment in the label:

1. **The environment cannot be forged.** The browser guarantees that a
   document's origin is its label, and the relay binds each channel by the
   origin of the message that asked for it ([Security](#security)).
2. **Storage partitions as Chrome's do.** Chrome keys storage by (origin,
   top-level site, cross-site ancestor): `bank.example` embedded in
   `evil.example` has its own label and storage, while a page and its own
   same-origin iframe share an environment and a label and reach each other
   as they do natively.

A label does not say whether a document is the top frame of a preview tab:
the runtime sees that (its `parent` is the Demi page), and the engine infers a
logical top-level navigation when the target has no cross-site ancestor and
the top-level site is its own. A same-site nested frame is therefore
navigated as if it were the top, differing only in `Sec-Fetch-Dest`
(`document` rather than `iframe`) and rare SameSite cases.

How addresses map:

- Relative and root-relative addresses stay, except a module import's relative
  address, which resolves to an absolute address and maps (the result the
  browser would resolve), so that a `data:` worker's imports keep their
  `opaque` parameter ([Opaque origins](#opaque-origins)).
- An absolute address, same-origin ones written absolutely included, maps to
  its target's label. With the current document's environment (o, t, c) and
  target origin x:
  - same origin as the document: the document's label;
  - a logical top-level document navigating itself by a link, a form or
    `location`, `target="_top"` and `window.open`: (x, x's site, none);
  - anything else, subresources, iframes and navigations in nested documents:
    (x, t, c or x cross-site to o or t).
- A navigation to another label maps to that label's boot page,
  `https://<label>/__demi/v<N>/boot.html#to=<path>`, since the target origin
  may have no forwarder yet; a navigation within the label is a plain preview
  address.
- A label is a digest and cannot be reversed. The runtime keeps the labels and
  environments it has seen, those the engine lists in the document's boot
  data and those it mapped itself, and reads addresses back from them.
- Reading an attribute, an `<a>`'s parts or `toString()` back gives the
  address the page wrote, for mapped absolute addresses.
- Attributes of custom elements, those with a `-` in their name, do not map:
  the site defines what they mean, and Figma keeps non-network addresses in a
  custom element's `src`.
- An attribute selector on a URL attribute matches the DOM's value. Paths do
  not change, so `[href^="/docs"]` holds; a selector naming another origin,
  such as `[href^="https://x.test/"]`, becomes `:is(original, mapped)`, and
  reads back as written in `selectorText` and `cssText`.

## The preview domain service

Every Demi shares one preview domain, `demi-preview.dev` by default;
`DEMI_PREVIEW_DOMAIN` names another, which then runs the same service with a
wildcard DNS record and certificate. A domain under `.localhost` may carry a
port and is served over plain HTTP, which Chrome treats as a secure context
there; every other is HTTPS ([Backend](../backend/backend.md#configuration)).
The service is Demi's: its code is in this repository at
`services/preview-domain`, and CI deploys it to a Cloudflare Worker with KV
storage ([Preview domain deployment](../delivery/builds-and-releases.md#preview-domain-deployment)).

**Namespaces**, at the domain's root:

| Request | Does |
| --- | --- |
| `POST /api/v1/namespaces { origins }` | Creates a namespace and answers `{ namespace, secret, expiresAt }` |
| `POST /api/v1/namespaces/<ns>/renew` | Renews it |
| `PUT /api/v1/namespaces/<ns>/origins { origins }` | Replaces the Demi origins allowed to embed it |
| `DELETE /api/v1/namespaces/<ns>` | Deletes it |

- Every request but creation carries `Authorization: Bearer <secret>`.
- `origins` are 1 to 8 secure-context origins, each exactly an origin: HTTPS,
  or `http://localhost` and `http://127.0.0.1`, with or without a port. Their
  hosts hold only letters, digits, dots and hyphens, since they go into a CSP
  as written.
- A namespace lives 90 days. The backend registers its namespace when it first
  starts, with the origins it serves its pages on (its public URL, and in
  development the web dev server's, `DEMI_PREVIEW_ORIGINS`), keeps the
  namespace and its secret as a control record
  ([Storage](../backend/storage.md#control-records)), renews it a day after
  its last registration or renewal, and replaces its origins when they
  change. A namespace the service answers 410, 404 or 401 for is gone, and a
  new one is registered, as is one whose stored secret no longer opens; any
  other failure is retried with waits that double, and the backend serves
  meanwhile, with previews unavailable. `backend-user-shard`'s `preview`
  module owns this. A namespace is never reused,
  since a browser may still hold its previous owner's storage.
- Creation is rate-limited by source address. Nothing checks that the origins
  belong to whoever registers them: registering another's origin only lets
  that Demi embed your namespace, which harms nobody.

**Every answer under a namespace** (`<ns>--<label>`) carries
`Content-Security-Policy: frame-ancestors <the registered origins>
https://*.demi-preview.dev`, so only the registered Demi pages, and preview
frames inside them, can embed the namespace, since the browser checks every
ancestor; and `Referrer-Policy: same-origin`.

**Paths:**

- `/__demi/v<N>/boot.html`, `boot.js`, `client.js`, `sw.js` and `policy` are
  versioned static files, never changed or removed once published. A Demi page
  uses the version it was built with. `sw.js` carries `Service-Worker-Allowed: /`.
- Any other path arrives only before the forwarder is installed: a document
  navigation gets the newest version's boot page, with that address as its
  target, and anything
  else 404. The service never logs paths, queries or bodies, and never serves a
  script there, so a site's own service worker registration always fails.
- The root has a page that says what the domain is for and how to report
  abuse.

Until the domain's public suffix list entry is accepted, a preview origin can
write a cookie for all of `demi-preview.dev`; the engine does not use the
browser's cookies, so this reaches only a site's own native writes.

## Opening and navigating

**Opening a tab of your browser**: the page asks the engine for the label of
the address's top-level environment and opens the iframe on
`/__demi/v<N>/boot.html#to=<path>`. The target is in the fragment, which the
preview domain never sees.

**The boot page**:

1. stops if it is a top-level window: a preview opens only inside Demi;
2. registers the forwarder with scope `/` and waits until it controls the page;
3. asks the Demi page for a channel through the client script and hands it to
   the forwarder;
4. tells the forwarder (target, token if any, its own `document.referrer`),
   waits for its acknowledgement, and replaces itself with the target. With no
   target in the fragment, the boot page's own address is the target.

**Navigations within an origin** (links, forms, `location`, reload, back and
forward) go through that origin's forwarder, never the boot page.

**Navigations to another origin** go to the target's boot page:

- A GET navigation needs no runtime: the frame opens the boot page, which
  announces (target, its own `document.referrer`) to the forwarder. The
  browser fills the referrer by the initiating page's policy, which the page
  cannot change, and the engine takes the initiator from it.
- A form submission other than GET: the runtime stops it and hands the
  request to the Demi page (method, target, body, and the initiating label,
  from the message's origin), which keeps it and answers a token; the frame
  opens the boot page with `#token=…&to=…`; the forwarder attaches the token
  to the navigation; the relay exchanges it for the kept request. A body
  never passes through the preview domain.
- `target="_top"` and a logical top's `window.open` are carried out by the
  Demi page at the runtime's request.

**New windows**: `window.open`, links with `target="_blank"`, and clicks with
Command, Control or Shift open a new tab of your browser on the same Host,
started by the opener's environment, beside its opener. `window.open` returns a
window object at once whose `postMessage`, `location` assignment, `close()`
and `closed` act on that tab through the Demi page; the new tab's
`window.opener` is such an object too, its `postMessage` reaches the opener,
`event.source` is the other window's object and `event.origin` its logical
origin, and the Demi page checks `targetOrigin` by logical origin. A sign-in
pop-up that returns its result by `postMessage` therefore works; one that polls
`popup.location.href` does not, since the two tabs cannot reach each other's
documents.

The preview iframe is sandboxed: scripts, same origin, forms, pop-ups and
modals are allowed, top navigation is not, so a page cannot navigate the Demi
page away. A nested frame's `target="_top"` link and `window.open(…, '_top')`
navigate the preview tab's top frame through the Demi page.

## The forwarder and the relay

**The channel** between a forwarder and the Demi page is a `MessagePort`. The
forwarder keeps it in memory; when the browser stops the forwarder, the next
request finds no channel, and the forwarder sends `need-port` to a document of
its origin, whose client script asks the Demi page for a new one. A navigation
with no document to ask gets a small page that connects and reloads. When the
Demi page closes, the channel closes and the forwarder gets `close`.

**A request** from the forwarder to the relay carries what the browser gave:
`url`, `method`, `headers`, the body, read whole as an `ArrayBuffer` since a
stream cannot be transferred to the page in every browser, `mode`, `destination`,
`credentials`, `redirect`, `referrer`, `referrerPolicy`, and the `token` and
`announcedReferrer` the boot page announced for a navigation. The answer is a
status, headers and a body the relay pulls one chunk at a time on the
forwarder's `pull`, as the direct channel's service worker does
(`packages/web/src/direct/service-worker.ts`); the browser's cancellation sends
`cancel`. The engine returns redirects, and the forwarder hands them to the
browser with `Response.redirect` to follow.

The forwarder adds `frame-ancestors`, from `/__demi/v<N>/policy`, which it
keeps in memory only (a page could read Cache Storage), to every navigation's
answer, and answers no top-level document request. Violation reports
(`destination` `report`) are not forwarded; they go to the Demi deployment that
set the policy.

**What a page can read.** A response a service worker gives is neither
filtered by CORS nor made opaque by the browser (measured): the page always
reads it whole. Natively, another origin's image, script, stylesheet and a
`no-cors` `fetch` reach a page opaque: usable, not readable. The forwarder
cannot make that: an opaque response comes only from a real cross-origin
network address, and a paired device behind NAT has none the browser could
fetch from. So the preview takes away what opacity protects instead:

- another site's content returned as the user, since a cross-site `no-cors`
  request carries no cookie ([Cookies](#cookies)); and
- content only the user's network reaches, since a page cannot request a more
  private network than its own ([Local network](#local-network)).

On top of that, every answer comes back through the forwarder and can travel
the direct channel. For a request to another label, the forwarder filters a
`cors` answer's headers by CORS (the CORS-safelisted ones and those
`Access-Control-Expose-Headers` names), and a `no-cors` answer's headers as for
a CORS request without credentials. What this costs is in
[Known differences](#known-differences).

**The relay**, in the browser plugin's page package:

- accepts messages from origins of its deployment's namespace only, by
  `event.origin`;
- binds each channel to the label of the origin that asked for it, and finds
  the preview tab it belongs to by `event.source` in the Demi page's frame
  tree, which names the Host; whatever the forwarder writes in a message
  afterwards, the engine takes that label as the receiving environment;
- maps preview addresses back to real ones: the label-to-environment
  mappings come from the engine, which lists in each answer the labels it
  computed, and from the runtime, which registers the ones it mapped; the
  relay computes each again before keeping it. A request whose label is not
  registered yet waits for it, and fails as a network error after 10 seconds.
  The engine always receives real addresses and environments and keeps no
  label;
- keeps cross-origin navigations' requests, and gives a token only to the
  target label's channel;
- sends each request on the tab's `preview` stream, which travels the direct
  channel when the Host has one and the relay otherwise
  ([Direct channel](../execution/direct-channel.md)), with the same frames
  both ways;
- opens a page's WebSockets: they do not pass a service worker, so the
  runtime's `WebSocket` asks the Demi page through the same bound channel, and
  the engine connects upstream from the Host.

**The tab and its top document.** The runtime of a preview tab's top document
tells the relay the page's address, title, icon and history state as they
change (`tab-page`), each move in its history (`tab-entry`, with the entry's
key and how it was reached), and that the page is leaving (`tab-leaving`),
and takes the address bar's Back, Forward, Reload and Stop as `tab-command`,
so the strip and the address bar show the page as a browser's do. The relay
keeps the tab's entries across the page's origins and enables Back and
Forward from them: within one origin Back goes through the page's Navigation
API, and to another origin's entry through `history.back()` and `forward()`
of the frame's session history. An entry the Demi page itself added after the
frame's would be stepped instead; the page adds none while a preview tab
navigates, a limit to lift if it changes. The relay reaches the
page's host through `PageHost.preview`, the `PreviewPlace` the shell gives the
plugin: the deployment's scheme, domain and namespace, and the runtime's
address in the web app's build.

**The runtime's delivery.** The runtime, with the rewriter's WebAssembly, is
about 2.4 MB, and every preview document loads it; fetched by each preview
origin, every new site would cost 2.4 MB more. A rewritten document loads it
from `/__demi/page/runtime/<release>.js`, naming the release of the engine
that rewrote it, since the rewriter's output calls the runtime of its own
release; the name is the workspace version, without the build metadata a
development build adds. The forwarder forwards that request as any other, and the relay
answers it with the runtime the web app's build carries, from the browser's
cache after the first time. A server release binds its command packages
exactly ([Bind an exact package](../execution/native-runtime.md#bind-an-exact-package)),
so the page and the Host's engine are of one release; a request naming
another release, as while a device's runner updates, fails as a network
error, and the page loads again once the Host has the release.

## The stream

The relay and the engine talk over the `preview` user stream of
`plugin-browser`, bound to the `browser.preview` operation of `demi-browser`,
one stream per conversation whose page shows a tab of your browser. It is
framed as the live view's stream is ([Framing and versions](live-view.md#framing-and-versions)):
a four-byte length, a one-byte kind and the payload, at most 16 MiB. Control
messages are JSON whose types are defined once, in
`command-package-browser-protocol`, generated into `@demicodes/plugin-browser`;
bodies are binary frames. Several requests share the stream, each with an id
the relay chooses.

| Kind | Sender | Payload |
| --- | --- | --- |
| `hello` | Relay | `{ scheme, domain, namespace, host }`: the first message, which labels need, since a user stream takes no arguments; the scheme is the one the product state carries |
| `request` | Relay | `{ id, environment, request, client }`: the receiving environment the relay bound; the forwarder's request with real addresses, which also holds the initiator the relay resolved (an environment, or null when unknown), whether the user started it, and whether body frames follow; and the client description ([Upstream requests](#upstream-requests), [Mobile](#mobile)) |
| `labels` | Relay | `{ id, environments }`: at most 256 environments the runtime mapped itself, for the engine to compute their labels; the engine answers `labels` with `{ id, labels }`, and the relay keeps only what the engine computed, so labels have one implementation |
| `request_body` | Relay | `id`, then up to 256 KiB of the request's body, a window ahead of the engine's `pull`s; an empty one ends it |
| `response` | Engine | `{ id, status, headers, labels }`: the head of the answer, and the labels its rewriting computed, each with its environment |
| `pull` | Either | `{ id }`: send the next chunk of that body |
| `chunk` | Engine | `id`, then up to 256 KiB of the body; an empty one ends it |
| `cancel` | Relay | `{ id }`: the browser gave up on the request |
| `failed` | Engine | `{ id, reason }`: the request failed before or during its answer, or the engine gave it up; the forwarder answers a network error |
| `socket_open` | Relay | `{ id, environment, url, protocols, client }`; the receiving environment is the socket's initiator |
| `socket_opened` | Engine | `{ id, protocol, extensions }` |
| `socket_message` | Either | `id`, a text or binary flag, then the message |
| `socket_close` | Either | `{ id, code, reason }`; a socket that could not open closes with 1006 |
| `state_take` | Relay | `{ id, tab }`: the agent's tab whose state a tab of your browser opens with; the engine answers `state` with its storage, after moving the agent's browser's cookies into the jar |
| `state_keep` | Relay | `{ id, token, origins, storage }`: the state of a tab of your browser, with the origins of its documents, for Open in Agent's Browser, under the token the page chose when it created the new agent tab at once; the engine answers `state_kept`, and `browser.handover` waits up to 10 seconds for the state of the token its tab carries, opening the address alone without it |

The relay, which holds the labels and the kept requests, resolves each
request's initiator from them ([The preview engine](#the-preview-engine)) and
maps addresses back, keeping the `__demi_*` parameters the engine reads; the
engine receives only environments. A socket's messages have no `pull`: a
WebSocket's upstream is read as fast as it sends, and a slow page then holds
the whole stream, a limit to lift if pages show it.

Both bodies run the same window. The engine sends the first four chunks of
an answer's body, up to 1 MiB, without waiting, and one more for each `pull`
the relay sends as the page reads one, so an answer up to 768 KiB arrives
whole with its head in one round trip,
which a far relay makes the cost that matters, while a slow page still holds
the Host back at most 1 MiB ahead, as the live view's stream does
([Backpressure](live-view.md#backpressure)); the relay keeps chunks that
arrive before the page reads them. The relay likewise sends a request's
first four body frames, its end among them when it fits, with the request,
and one more for each `pull` the engine sends as it takes one, so a small
body, such as each `document.cookie` write a page makes, costs no round
trip beyond its answer's. A body frame past the window ends the stream. A tab shows loading from the click of Reload, Back or
Forward, until the next document loads or the history moves within the
document, as a browser's does. A frame the protocol refuses ends the
stream, as on the live view's, and the relay answers every open request with a
network error and opens the stream again.

Page state travels on the stream rather than in tab methods, since a page
call's body is limited to 1 MiB and a runner's message to 4 MiB; a tab that
has state to move implies a running Host, so the stream's never waking a
stopped Cloud costs nothing there. The tab methods of the browser plugin
carry what is not a request: the top-level label of an address the user
opens, the `preview_open` method,
which runs the `browser.preview_open` operation and answers `{ label,
environment, origin }`, or `invalid_input` for an address that is not a web
address, the state moved between the two
browsers ([Page state](#page-state)), and opening a page in the agent's
browser. They wake a stopped Cloud as a tab method that opens a tab does; the
stream never wakes it ([User streams](../product/web-api.md#user-streams)).

## The preview engine

The engine is the `command-package-browser-preview` crate, composed into
`demi-browser`. One engine serves every conversation of the Host; it keeps
nothing per conversation but its open requests.

**Input**: the receiving environment, from the relay's binding of the channel,
never from the page; the request; and its initiator, which the relay resolves
from what it holds, in order:

1. the kept request a token names: the label that started it;
2. a subresource or a fetch: the receiving environment itself;
3. a navigation: the `announcedReferrer` of the boot page, or else the
   request's `referrer`. When its origin is a label of the deployment's
   namespace, that label is the initiator; otherwise the initiator is unknown,
   and counts as cross-site.

The third rule matters when a page that is same-site with Demi, such as
another site of the same company, learns a preview address and embeds it: it
shares Demi's storage partition and so the installed forwarder. Its
navigations have no known initiator and carry only `SameSite=None` cookies,
and `frame-ancestors` blocks their answers, as a real browser blocks a
cross-site iframe.

**Addresses**: rewriting maps absolute addresses to preview addresses and
lists the labels it computed in the answer, for the relay to register.

### Cookies

Cookies never reach the user's browser. The engine keeps one cookie jar per
Host, so per (user, Host), shared by every tab of your browser of that user
on that Host, across conversations, and kept on the Host in `demi-browser`'s
data directory, so a sign-in outlives restarts as a browser profile's does.
Storing and matching (domain, path, expiry, `Secure`) is the `cookie_store`
crate's; the rules a browser adds on top are Chrome's (RFC 6265bis): `SameSite`
defaults to `Lax`, `HttpOnly` is invisible to scripts, the `__Secure-` and
`__Host-` prefixes, and refusing a public suffix domain. The `Cookie` header
orders by path length, then by creation; replacing a cookie keeps its creation
time.

- An upstream `Set-Cookie` goes into the jar and never to the browser.
- The SameSite context comes from the receiving environment and the
  initiator. A logical top-level navigation is `strict` when the initiator is
  same-site with the target, otherwise `lax` for a safe method and `none` for
  others; a preview tab's first load counts as the user's input, `strict`.
  Any other request is `strict` when target and initiator are same-site with
  the top-level site, and `none` otherwise.
- Credentials follow the forwarder's `credentials`. A cross-site `no-cors`
  request, another site's image, script or stylesheet, or a `no-cors`
  `fetch`, never carries a cookie, as in Safari by default: its answer is
  readable ([What a page can read](#the-forwarder-and-the-relay)), and a
  cookie would hand the page another site's content as the user.
  Navigations, iframes and credentialed CORS requests are unaffected: an
  iframe is another preview origin the browser isolates, and the engine
  refuses a CORS request upstream does not allow.
- `document.cookie` is the runtime's: the document's boot data carries the
  non-`HttpOnly` cookies its address sees, reads use that local view, and a
  write updates it at once and goes to `/__demi/host/cookie` of the document's
  origin by `fetch`, through the forwarder. Chrome's synchronous XHR does not
  pass a service worker (measured), so a write cannot complete synchronously:
  the relay handles one channel's requests in order, and requests arriving on
  the channel after a cookie write wait for it, so the next `fetch` carries
  the cookie. When a `fetch` or XHR answer changed cookies, the engine adds
  `x-demi-cookie-changed`, and the runtime refreshes its view before the next
  read; a read before that refresh sees the old value.

### Local network

A page cannot request a more private network than its own, by the rules of
Chrome's Local Network Access. Addresses are, from public to private: public;
local network (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`,
`100.64.0.0/10`, `169.254.0.0/16`, `fc00::/7`, `fe80::/10`); and loopback
(`127.0.0.0/8`, `::1`, `198.18.0.0/15`, `0.0.0.0`).

- The target is judged by the address the engine connects to: a name is
  resolved, the addresses more private than the initiating page are dropped,
  and a request with none left is refused before it is sent, so a public page
  cannot trigger an action on a local device with an `<img>`.
- The initiating page is judged by the address the engine fetched it from,
  never by resolving its name again: a site may point its name at `127.0.0.1`
  after its page loaded. `localhost` and literal addresses are judged as
  written. A page the engine has no record of, as after its restart, is
  judged by the most public address its name resolves to now.
- A navigation whose initiator the browser does not give (from an
  `about:blank` frame, or a `no-referrer` link) is judged by the preview tab's
  top-level site: a local development page's tab can reach the Host's loopback,
  a public site's cannot.
- The rule applies to every request with a known initiator: subresources,
  navigations and iframes a page starts, and WebSockets. An address the user
  opens is not limited. Where Chrome asks the user, the preview refuses.

For example, a development page at `http://localhost:5173` requests
`http://localhost:8000` as usual; `https://evil.example` requesting
`http://192.168.1.1/` or `http://localhost:5432` is refused.

### CORS, CORP and Referer

The browser checks CORS against the real origin, a label; the engine against
the logical origin. When initiator and target differ in logical origin and the
mode is `cors`, the engine requests upstream with the logical `Origin`,
preflighting a non-simple request, and decides by upstream's
`Access-Control-Allow-Origin` and `-Credentials`; when upstream allows, it
replaces both with the initiator's label origin, which the browser then
accepts. CORP is judged by logical origin the same way.

The engine replaces preview addresses in `referrer` with real ones, under
`referrerPolicy`; a cross-origin request with no `referrer` gets one from the
initiator's logical origin and policy, since some image hosts refuse requests
without one. A site's `Referrer-Policy: no-referrer` reaches the browser as
`same-origin`, or the origin's own navigations would have no referrer and an
unknown initiator; the engine remembers the document's policy and drops the
`Referer` upstream as it says.

### Response headers

A site's CSP and `X-Frame-Options` are removed, and the preview's policy
replaces them: `default-src https://*.demi-preview.dev data: blob:
'unsafe-inline' 'unsafe-eval'`, with the preview domain's own scheme and
port. An address the
rewriting missed, or a request straight to another site, is then blocked
before it leaves the user's browser.

A page's own synchronous XHR does not pass the service worker (measured: it
reaches the preview domain), and the browser fetches synchronously only from a
network address, so the preview does not support it: the runtime fails it as
a network error, throwing `NetworkError` from `send()`. Synchronous XHR is
deprecated and rare. A site's `Permissions-Policy`, and `Feature-Policy`,
which Chrome reads without it, reach the browser without their `sync-xhr`
entry: the runtime reads the page's own `blob:` and `data:` scripts with a
synchronous request to rewrite them, and Cloudflare's challenge frame both
forbids synchronous requests and runs its code from a `blob:` script.

### Rewriting

The engine rewrites navigations' HTML, stylesheets, and scripts whose
`destination` is `script`, `worker` or `sharedworker` (a worker's own script
and the modules it imports). A worker's own script, which the runtime marks,
gets one line first that starts the runtime ([Storage and browser
features](#storage-and-browser-features)). A worklet's script passes
unchanged: there is no runtime there.

The rewriter is one implementation, in Rust: addresses, JavaScript (parsed
with oxc, with scope analysis to tell whether `location`, `top` or `eval` are
unbound, and patched in place so untouched bytes and positions stay),
CSS (cssparser's tokenizer finds `url()`, `@import` and `image-set()`),
HTML (lol_html, streaming; attribute values are decoded from entities first,
or `style="--logo:url(&quot;…&quot;)"` and `srcdoc="&lt;img …&gt;"` hide their
addresses), attributes and import maps. The engine runs it natively; the
runtime runs the same code compiled to WebAssembly, synchronously, for what
the page generates itself. Results are cached by source content, independent
of the document's environment, with the mapped preview addresses kept as
placeholders filled in per answer. For a debugger, the engine fetches the
upstream source map when asked and composes it with the rewriting's, recorded
at word boundaries.

**Integrity**: rewritten bytes always fail Subresource Integrity. The
rewriter hands `integrity` to the engine, which checks it against the
upstream bytes and aborts the answer when they differ; the browser checks
nothing more.

### Opaque origins

A worker started from a `data:` address has an opaque origin. The requests it
maps carry an `opaque` parameter, and the engine sends them with a `null`
initiator (`Origin: null`, cookies and `Sec-Fetch-Site` as cross-site), and
maps the modules it imports the same way, so the whole module graph requests
as `null`. The worker actually runs on its starter's preview origin, which the
browser checks CORS against, so an allowed answer's
`Access-Control-Allow-Origin` is that origin.

### Upstream requests

Defenses such as Cloudflare's decide at their edge, by the TLS and HTTP/2
fingerprint and the request headers, before any page loads. The engine
requests with wreq on BoringSSL, emulating the browser the user's User-Agent
names, Chrome 154 by the closest version wreq has, Chrome 149, with the
headers the user's browser sent, in the order it sent them; the values the
engine computes (`Cookie`, `Origin`, `Referer`, `Sec-Fetch-*`) go where the
browser put them. wreq without an order would sort them by its emulation's
built-in order, which differs from Chrome's. A preview tab's first load counts
as typed into the address bar, with `Sec-Fetch-User: ?1`.

Chrome 154's handshake differs from wreq's latest emulation in two places,
which the engine adds for Chrome 154 and later: the trust anchors extension
(0xCA34, Chrome's list of trust anchor ids), with BoringSSL's
`SSL_CTX_set1_requested_trust_anchors`; and a GREASE value and ML-DSA-44, 65
and 87 at the head of the signature algorithms, with a BoringSSL patch that
lists algorithms in the ClientHello ahead of those used for verification. No
public CA issues ML-DSA certificates, and this BoringSSL does not implement it,
so ML-DSA takes no part in verification. wreq and btls-sys are therefore
vendored with the patches ([Vendored crates](../delivery/builds-and-releases.md#vendored-crates)).
Compared on tls.peet.ws with the same Chrome directly, the TLS (JA4,
peetprint, each extension's content), HTTP/2 settings and priorities and the
header names and order are identical, but for the GREASE values each
connection draws.

The engine reuses idle connections. A server may close one just as a request
goes out, with no answer at all; Chrome sends such a request once more on a
new connection, and so does the engine for a GET, HEAD or OPTIONS without a
body whose connection failed to open or closed before the answer completed.
The engine cannot tell a reused connection from a new one, so it also retries
once where Chrome would report `ERR_EMPTY_RESPONSE`; such servers are rare.

A challenge must be one the user can pass: Cloudflare's checkbox, Turnstile
and reCAPTCHA run in the preview, and a passed challenge's cookie goes into the
Host's jar. Google's `x-browser-validation` and similar integrity headers are
not emulated: forging them is defeating bot detection.

## Storage and browser features

Each label is a real origin, so these are the browser's own, partitioned by
(Demi's site, label), and kept across reloads:

| Feature | Handling |
| --- | --- |
| localStorage, IndexedDB, Cache Storage, Web Locks, BroadcastChannel | Native |
| sessionStorage | Native; two preview tabs of one site in one Demi tab share it, a known difference |
| Worker, SharedWorker | The runtime starts before the worker's script: the rewriter writes the script's first line, with the boot data, the wait for the channel and the runtime's load (`importScripts` for a classic worker, `import` for a module, since a module's imports run before its body). For a worker from an address, the page's runtime only maps the address and marks whether it is a classic or module script (the browser requests both alike), and the engine adds the line when it rewrites; the address stays, so SharedWorker instances still differ by address and name. For a worker from `blob:` or `data:`, which the engine never sees, the page's runtime reads the source, rewrites it, adds the line and starts it from a new `blob:` address. A SharedWorker's copy is kept by (original address, type) in the origin's topmost window, so frames using one address reach one instance; revoking the original revokes the copy, existing instances keep working and new ones fail to load, as natively |
| A worker's channel | The document that starts a worker sends it, for WebSockets and label registration, as the worker's first message, or for a SharedWorker as the first message on each connection's port, the first to arrive taken. The first line takes it before the site's code sees it, so a SharedWorker's port starts at connection, and messages arriving before a site sets `onmessage` long after connecting are lost, where natively they queue |
| Worklet | Native, unrewritten: there is no runtime there. It inherits the page's CSP, so a worklet importing another origin's module fails, a known difference |
| Same-origin frames, a page's own `srcdoc` frames | Native |
| A page's own `about:blank` frame | Its window and document are reachable natively, but Chrome does not let the forwarder control it (measured): its requests reach the preview domain. When the page first reaches such a frame's window or document, the runtime installs a whole runtime in it, with its own channel, so addresses, `location`, forms and messages there behave as in the page, and replaces the frame's `fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource` and `sendBeacon` with the parent's. The frame's own element loads, such as an image inserted into it, pass uncontrolled and get 404 |
| Cookies | The engine's jar ([Cookies](#cookies)) |
| A site's own service worker | Unsupported: it would replace the forwarder. The runtime offers an object of the same shape whose operations reject; a site bypassing the runtime gets no script from the preview domain, and the registration fails |
| Notification permission | Chrome refuses it in cross-origin iframes; the runtime answers `default`, never asked |
| Camera, microphone, clipboard, full screen | The Demi page delegates them with `allow` on the preview iframe; the browser's prompt names the preview origin |
| A site's own CSP | Replaced by the preview's ([Response headers](#response-headers)); `blob:` workers and a page's `about:blank` frames inherit the preview's |

## The runtime

The runtime, `@demicodes/preview-runtime`, runs in every preview document
before the page's scripts:

- **Addresses**: maps absolute addresses and reads them back (attributes,
  `<a>`'s parts, attribute selectors on URL attributes, `MediaMetadata`
  artwork); rewrites what the page generates itself (`eval`, `new Function`,
  `innerHTML`, `document.write`, `DOMParser`, `data:` and `blob:` scripts,
  `javascript:` URLs) synchronously with the rewriter's WebAssembly. A
  `<script>`'s text is rewritten when the element is in the document or being
  inserted, the only times the browser runs it; text that does not parse stays
  as it is, since pages keep shaders and templates in non-JavaScript scripts.
- **Labels**: the relay pushes the labels of each answer to the document, so it
  knows them before reading those addresses back.
- **Origin**: `self.origin`, `location`, `document.URL`, `document.baseURI`,
  `document.referrer`, `document.domain` and `location.ancestorOrigins` give
  logical values, destructured reads as member reads.
- **Hierarchy**: a logical top document's `top` and `parent` are itself; a
  nested document's `top` is the logical top. The real top is the Demi page.
- **Navigation**: cross-origin navigations and new windows as
  [Opening and navigating](#opening-and-navigating) says.
- **Messages**: `postMessage`'s `targetOrigin` names a logical origin, while
  the target window's real origin is a label, possibly one of two (top or
  nested environment). The runtime sends to `*` with (logical sender origin,
  target origin) wrapped around the message; the receiving runtime checks the
  declared sender against `event.origin`'s label, from the boot data or by
  asking the engine for an unknown label while queueing messages in order,
  drops the message when either does not match, and gives the logical sender
  as `event.origin`. A message to another realm of the same origin (a page's
  own blank or `srcdoc` frame) is wrapped by the sender's runtime and sent with
  the sender realm's native `postMessage`, since the browser takes the
  caller's window as `event.source` (measured).
- **WebSockets** and **cookies**, as above; the Cookie Store API too.
- **Appearance**: the boot document's three scripts (client script, boot data,
  runtime) are removed when the runtime starts; replaced functions keep their
  native `toString`, `name` and `length`; `performance` entries use logical
  addresses and omit the runtime's own requests; the runtime's globals are not
  enumerable. `Error.prepareStackTrace` is never set, since pages that see it
  call it.
- **`requestIdleCallback`**: a cross-origin iframe gets little idle time from
  Chrome, so a callback that has not run within 50 ms runs from a timer with a
  20 ms budget.
- **Source maps**: a network script's map is fetched only when a debugger
  asks for it.

## Page state

**Open in Your Browser** and **Open in Agent's Browser** take one snapshot of
the page's state at that moment, and afterwards each browser keeps its own,
as Playwright's `storageState` and Chrome's Duplicate Tab carry state. Nothing
is synchronized later: two browsers changing one value would need merge rules,
and the agent's browser goes with its conversation's release.

**What moves**: for each origin in the tab's frame tree, its localStorage,
sessionStorage and IndexedDB, and the cookies of those origins' sites. In the
target browser the values of those origins and sites are replaced by the moved
ones. Since the cookie jar is shared by every tab of your browser on the Host,
a page the agent signed in to with a test account replaces the user's own
sign-in on that site after Open in Your Browser: that is what sharing the jar
means, and nothing isolates it.

**State must be in place before the page's first script**, or an
application decides it is signed out:

```text
agent's browser ──▶ your browser
  demi-browser reads over CDP: Network.getAllCookies on the tab's session (its
  browser context), and localStorage, sessionStorage and IndexedDB in each frame
  cookies ──inside the Host──▶ the engine's jar, as Set-Cookie with their attributes, HttpOnly included
  storage snapshot ──▶ the relay, kept for the new tab's labels
  each origin's boot page asks the relay for its label's snapshot, writes it,
  waits for IndexedDB to finish, then replaces itself with the target

your browser ──▶ agent's browser
  the relay asks the runtime in each of the tab's frames for its own storage (same origin)
  the engine lists the jar's cookies of those sites
  demi-browser: Network.setCookies; for each origin, intercept a blank document with Fetch,
  write the storage, then navigate to the target
```

- **One codec** reads and writes the storage on both sides: the preview
  domain's `/__demi/v1/state.js`, which a hidden frame on the preview origin
  runs in the user's browser (`state.html`, `state-frame.js`), sharing the
  origin's storage, sessionStorage included, and which `demi-browser` runs in
  an isolated script context over CDP in the agent's browser, so the page
  never sees it. The agent's browser takes the state with the
  `browser.handover` operation, which sets the jar's cookies and writes the
  storage into a blank document on the origin before navigating; a reload or
  a retry opens the address alone.
- **IndexedDB values** keep their structured-clone types with a tagged
  encoding: `Date`, `ArrayBuffer`, typed arrays, `Blob`, `Map`, `Set` and
  `BigInt`. A value that cannot be serialized, such as a `CryptoKey`, is not
  moved, and the new tab's toast names the origin and store it skipped.
- **Frames of other origins** in the agent's browser: Chrome partitions a
  third-party iframe's storage by its top-level site, so writing it would
  need a context that embeds that origin in that top. The first version moves
  the top-level origin's storage and every cookie, and says nothing of
  iframes; a tab of your browser has no such limit, since each origin's boot
  page runs in its own environment.
- **Partitioned cookies**: the jar does not know `Partitioned`, so a moved
  partitioned cookie becomes an ordinary one.
- **Not moved**: Cache Storage, service worker registrations (a preview
  supports none), OPFS.
- **Size**: a snapshot is at most 16 MiB; a larger one moves cookies only and
  says so in a toast.

## Presenting a page

`demi browser present <tab>` tells Demi that the user should see a tab of the
agent's browser. It prints the tab's title and address; it changes nothing in
the browser, and opens nothing in the panel. The command's block in the
transcript shows a card for each page its command presented, from the shell
view's `presented` field ([Transcript](../agent/runtime.md#transcript)), with
the page's title, address and Open. Open takes the agent's tab's state as Open
in Your Browser does, when the tab still exists, and only its address when the
tab is gone, as after the conversation's release; it opens the work panel
when it is closed. The agent shows a page the user should look at with
`present`, and keeps `--show` ([Showing a tab](live-view.md#showing-a-tab))
for the live view of a tab it wants the user to watch.

## Mobile

Mobile in a tab of your browser makes the page a phone's in two layers:

- **Requests**: the relay sends the tab's client description as an Android
  Chrome: the User-Agent `Mozilla/5.0 (Linux; Android 10; K)
  AppleWebKit/537.36 (KHTML, like Gecko) Chrome/<the user's version>.0.0.0
  Mobile Safari/537.36`, the user's browser's brands, `mobile: true` and the
  platform `Android`. The engine chooses the Android fingerprint by that
  User-Agent, and the client hints follow. Bing serves its phone pages from
  this alone.
- **Scripts**: the engine adds `device: { userAgent, mobile, platform }` to the
  document's boot data (not `client`, which names the client script), and the
  runtime overrides, before the page's scripts, `Navigator`'s `userAgent`,
  `appVersion`, `platform` (`Linux armv81`) and `maxTouchPoints` (5), and
  `NavigatorUAData`'s `mobile` and `platform`. JD.com, which redirects to its
  phone site from a script that reads the User-Agent, needs this layer too.
- **Frame**: 390 × 844, scaled to fit the panel and centred;
  `devicePixelRatio` stays the user's own, as the live view's Mobile does. The
  phone's size has one source, `live::PHONE` in
  `command-package-browser-protocol`, which the page receives as
  `PHONE_WIDTH` and `PHONE_HEIGHT`, and the live view's Mobile uses too.
- **The mode moves with the page**: the panel tab keeps its `mobile` flag;
  `state` says whether the agent's tab was in Mobile, so a tab of your
  browser takes the mode before its first request, and `browser.handover`
  takes `mobile` and puts the agent's tab in Mobile before it writes the state
  and loads the page.

Not covered yet: `getHighEntropyValues()`; `navigator` in workers, which keep
the user's own, since a worker's boot line comes from the engine's cache,
which would have to key on the client; the `pointer` and `hover` media
features; `screen`'s size; and touch events.

## Security

| Threat | Protection |
| --- | --- |
| A previewed site reads Demi's cookies, storage or page, or calls Demi's API as the user | The preview domain is another registrable domain, not same-site with Demi |
| One previewed site reads another | Different labels are different origins, which the browser isolates; a label names one environment, and the engine serves it only that origin's content |
| Another web page embeds a preview address to read or change it | `frame-ancestors` allows only the registered Demi pages; Chrome's storage partitioning leaves other top-level sites without a forwarder |
| A page same-site with Demi embeds a known preview address to make the Host send requests through the user's open Demi tab | The initiator is unknown and cross-site for cookies, and the answer is not shown ([The preview engine](#the-preview-engine)); a loopback or local target is requested only when that environment's own top is in that network, that is, the user opened that local address in a preview |
| A previewed site reads another site's content as the user, through a `no-cors` subresource it can read | A cross-site `no-cors` request carries no cookie ([Cookies](#cookies)) |
| A previewed public site reaches services on the Host's local network or loopback | A page cannot request a more private network than its own, by the address connected to, refused before sending ([Local network](#local-network)) |
| A site's script asks the Demi page for a channel itself, claims its requests come from another site, or takes a navigation's answer | A channel binds to its message's origin; the engine takes the bound label as the receiving environment; a token goes only to its target label |
| A site registers its own service worker in place of the forwarder | The runtime refuses it, and the preview domain never serves a script for an unknown path |
| The preview domain sees user data | It receives only requests for its versioned static files; the Demi page keeps POST bodies; requests that reach it before the forwarder is installed are not logged |
| The preview domain is used for phishing | Content enters only through a registered Demi page; opened as a top-level page, it shows nothing |
| The preview domain's code is changed | The forwarder sees every preview's content, which is the price of trusting the domain: only this repository's CI publishes it, and a deployment can run its own domain |
| The domain expires and is registered by someone else, or the Cloudflare account is taken over | The operator's to prevent: whoever holds the domain or the account controls the forwarder, and Demi adds no protection of its own |

## Known differences

- One site in different environments (top level, embedded elsewhere) is
  different origins that cannot reach each other directly, as they natively
  can; this is rare.
- A site's own service worker, offline PWAs, push and background sync.
- Passkeys of the site's own domain (the RP ID does not match).
- Error stacks name preview addresses and include the runtime's frames.
- `referrerpolicy="no-referrer"` and `rel="noreferrer"` on an element used for
  a same-origin navigation count that navigation as cross-site for cookies.
- FedCM is unavailable: the identity provider sees the preview origin, not the
  site's registered one. Redirect-based OAuth is unaffected.
- A page's own synchronous XHR: `send()` throws `NetworkError`, and
  `readyState` stays 1, where Chrome's error leaves 4.
- Another origin's resources are readable ([What a page can
  read](#the-forwarder-and-the-relay)): a canvas with another origin's image
  no longer throws `SecurityError` on `getImageData`; script errors show their
  full message rather than `Script error.`; stylesheets' `cssRules` are
  readable; a `no-cors` `fetch` answer is readable, with type `basic`.
- Cross-site subresource requests carry no cookies, as in Safari: analytics,
  ad syncing and subresources that show a sign-in through third-party cookies
  behave as signed out.
- Subdomains of one site read each other's credentialed subresources, which
  natively they cannot; the risk is a site hosting another's content on its
  own subdomain.
- A public service that authorizes by source address only, such as one open to
  a company's outgoing address, is readable by any page opened in a preview
  when the Host leaves from that address.
- In a cross-origin iframe, `content-visibility: auto` content renders only on
  entering the viewport, with no margin, so it appears slightly late when
  scrolling.
- A Host behind a fake-IP proxy, such as Clash or Surge, resolves public names
  into `198.18.0.0/15`, which counts as loopback: every site is then a local
  page to the engine, as it is to Chrome in the same setup.
- A page's own image inserted into an `about:blank` frame it created does not
  pass the forwarder and does not load.

## Browser behavior the design relies on

Measured in Chrome 155, and checked again by the behavior cases
([Tests](#tests)):

| Behavior | Result |
| --- | --- |
| The parent's forwarder controls a `srcdoc` child frame | Yes |
| The forwarder controls a worker a page starts from `blob:` | Yes |
| The forwarder controls an `about:blank` child frame a page creates | No: its requests go to the preview domain |
| Synchronous XHR passes the forwarder | No: it goes to the preview domain |
| `navigator.sendBeacon` and `keepalive` fetches pass the forwarder | Yes |
| The browser filters a service worker's answer by CORS or makes it opaque | No |
| A module SharedWorker from an address applies its answer's CSP; workers from `blob:` inherit the page's | Yes |
| A worker's own script's requests | `destination` `worker` or `sharedworker`, `mode` `same-origin`, classic or module alike; module imports have the same `destination` with `mode` `cors` |
| `event.source` of a message sent with another realm's native `postMessage` | The caller's window |
| Viewport margin of `content-visibility: auto` in a cross-origin iframe | None |

## Tests

Automated tests reach no outside network ([Testing](../delivery/testing.md)).

- **Behavior cases**: the spike's lab, local fixtures each loaded directly and
  through the preview, is the preview lab suite,
  `packages/browse/src/lab/preview-lab.test.ts`: it starts a development
  backend with its echo model, the web app, a runner claimed through the API
  and the lab's sites on free ports, creates the conversation through the
  API, and drives Chrome through the relay as the page does. It runs when
  `DEMI_TEST_CHROME` names Chrome ([Validation](../delivery/builds-and-releases.md#validation)),
  `DEMI_TEST_PREVIEW_LAB_CASES` naming a subset, in about two minutes; the
  last runs: 192 cases agree, 30 differ by policy, 22 are unsupported by
  design, and one, a public page reaching loopback, needs a public site the
  lab cannot provide. Each keeps the direct load as its
  baseline; a deliberate difference states its expected value. The cases the
  spike added last are kept: no cookies cross-site, a public page cannot reach
  loopback, a request resent when a reused connection closes, a site that
  forbids synchronous requests, an address written in HTML read back after
  parsing, and the requests of a SharedWorker from an address.
- **Rewriter**: the crate's own tests, on its inputs and outputs.
- **Acceptance**, by hand or with `bun browse`, never in the automated suite:
  public sites, the development servers and hot reload of six frameworks,
  application interaction (Streamlit, Gradio, Jupyter), new windows,
  security, the fingerprint on tls.peet.ws, and challenges, which a person
  passes. The acceptance browser drops the automation marks
  (`navigator.webdriver`, the HeadlessChrome User-Agent, the small screen of
  cross-site iframes) or sites challenge it again and again. Results compare
  with the spike's: 192 cases agree, the six frameworks' hot reload keeps its
  state, 40 of 50 sites match.
- **Still to measure**: the cost of one round trip through the forwarder, the
  relay and the stream; ranges and streaming of large files and video.

## Open decisions

- Whether the jar and the tabs of your browser need more than they have across
  restarts: the jar is kept on the Host and the panel keeps its tabs, but a
  page's in-memory state is gone, as after any reload.
</content>
</invoke>
