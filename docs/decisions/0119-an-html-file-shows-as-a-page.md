---
status: proposed, superseded in part by 0131
---

# An HTML file shows as a page

## Problem

An agent writes `report/index.html` with a chart, a stylesheet beside it and
a script from a CDN. Alex opens it in the File view and reads its source:
the view shows HTML as text, and the backend serves an `.html` file as a
download. To see the page, Alex has to find the file on the Host and open it
in a web browser there, which a remote Host does not even have a screen for.

Showing it in an `iframe` straight from the file route would be worse. The
page would run on Demi's own origin with Alex's session cookie, so a script
in any HTML file, one in a cloned repository included, could call Demi's API
as Alex: send an agent a message, run a command on a Host, read any file.

## Decision

The File view shows an HTML file as the page it describes, in a sandbox that
gives the page no way into Demi. The same design serves every Host, the one
the web browser runs on included.

### What the user sees

```text
+-----------------------------------------------------------+
| HTML · 48 lines · 2 KB         [Preview|Source] [Download] |
+-----------------------------------------------------------+
|                                                           |
|   Quarterly report                                        |
|   [ chart drawn by the page's own script ]                |
|                                                           |
+-----------------------------------------------------------+
```

- An `.html` or `.htm` file opens in Preview, as Markdown and SVG do, and
  Source shows its text in the editor. A link to a line of it opens Source.
- The page draws on white, as a web browser draws a page without a
  background, whatever Demi's theme.
- Its stylesheets, scripts, images and fonts load from beside it, by relative
  paths, `../assets/app.css` included, as long as they stay in the workspace.
  A link to a folder loads its `index.html`, as a web server's does.
- Its scripts run, and reach other sites as a page in a browser tab does, so
  a page that loads a chart library from a CDN draws its chart. A hostile
  page could send what it can read, the workspace's files that are not
  hidden, to a site of its choosing; blocking it with `default-src 'self'`
  would leave such pages without their libraries.
- A link navigates the frame, as in a browser tab; a link
  that opens a new window opens a browser tab of its own; `alert`, forms and
  downloads work.
- When the file changes, the page loads again.
- A committed version in Changes and an attachment's copy show their source
  only: neither is a folder of files a page can load from.

What a page cannot do, as in any sandboxed frame:

- Use `localStorage`, IndexedDB or cookies: they throw `SecurityError`, so a
  page that needs them fails as it would in a private window that refuses
  them.
- Load a path that starts at the root, such as `/assets/app.css`: it names
  Demi's own server, not the workspace, and fails.
- Read a hidden file or folder, such as `.env` or `.git`, or a file outside
  the workspace, through a link or not.

### Design

- **A grant.** `POST /conversations/{id}/fs/html { path }` answers
  `{ url }`, such as `/api/html/<token>/report/index.html`: the file's place
  under the conversation's working directory, or 404 for a file outside it.
  The token is 256 random bits, which the backend keeps in memory with the
  session's token hash and the conversation. One token serves a session's
  conversation, so the web browser's cache holds across files. It ends with
  the session, which each request checks, and with the backend's process; a
  view asks for the URL each time it shows a page.
- **The HTML route.** `GET` and `HEAD /api/html/{token}/{*path}` read the
  file at the path under the conversation's working directory, as the
  session's user, outside the gate that checks the session cookie. The path
  is the URL's, so a relative URL resolves as a web server's would. A folder
  answers its `index.html`, after a redirect that adds the `/` its relative
  URLs need. Under `/api/`, the page's service worker leaves the frame's
  navigation to the network.
  - It answers 404 for a path with a component that starts with `.`, and
    for one whose real path, as the Host's `fs_realpath` answers it, leaves
    the root, so a symbolic link named `notes.txt` that points at
    `~/.ssh/id_rsa` reads nothing. The check reads the path as the Host
    names its paths, so on Linux and macOS a `\` is part of a name. It runs
    in the read's own admission, on the Host the read uses. Between the check
    and the read, a program on the Host could swap the file for a link and
    have the link's target read; such a program runs as the user and can
    read that file itself.
  - The request log names the route with `{token}` in place of the token,
    as it names a share's.
  - Every answer carries:

    ```text
    Content-Security-Policy: sandbox allow-scripts allow-forms allow-modals
      allow-popups allow-popups-to-escape-sandbox allow-downloads
    Access-Control-Allow-Origin: *
    Referrer-Policy: no-referrer
    X-Content-Type-Options: nosniff
    ```

    The sandbox gives the document an opaque origin even when opened on its
    own, so it holds no cookie of Demi's and reads nothing of the page
    around it. A request from it carries no cookie either, which is why the
    token is in the path. Module scripts and `fetch` from an opaque origin
    are cross-origin requests, which the `Access-Control-Allow-Origin`
    admits. The referrer policy keeps the token from the sites the page
    links to. No `allow-same-origin` and no `allow-top-navigation`: the page
    never reaches Demi's origin, nor replaces Demi's page.
  - A file is served as the media type of its extension among the web's own
    types (HTML, CSS, JavaScript, JSON, WebAssembly, SVG, images, fonts,
    audio and video, text), anything else as `application/octet-stream`.
    Range requests, versions and `ETag` work as on the file route.
- **The view.** `FileContents` gains:

  ```ts
  /** Where `path` loads as a web page; absent when its source serves none. */
  htmlUrl?(path: string): Promise<string>
  ```

  A file is a page by its media type, `text/html`, which the file-type table
  gains for `.html` and `.htm`, not shown in place: the file route still
  serves it as a download. Where its source has `htmlUrl`, `FileContent` offers
  Preview and Source for it and shows an `iframe` with the same `sandbox`
  flags at that URL on a white ground. It reads the file's text as for
  Source, and sets the frame's URL again when the text's version changes.
  Changes keeps showing HTML as text, as it has no `htmlUrl`.
- **The gallery.** The File view's specimen has an HTML file whose script
  responds to a click. The memory source's `htmlUrl` answers a `blob:` URL of
  its text, sandboxed by the frame's flags.
- An older page shows HTML as text, as before. No runner changes, and
  nothing is stored.

## Spike

Chrome, a server answering a document under `Content-Security-Policy:
sandbox allow-scripts`, and a `SameSite=Lax` session cookie set before. The
document was opened on its own and in an `iframe` of a page on the server.

- The document's navigation carried the cookie. Its stylesheet, scripts and
  `fetch` carried none, and `self.origin` was `"null"`. So a grant has to
  travel in the URL.
- A stylesheet and a classic script loaded. A module script and `fetch` of a
  JSON file beside it failed, and loaded once their answers carried
  `Access-Control-Allow-Origin: *`.
- `localStorage` and `document.cookie` threw `SecurityError`.
- Adding `default-src 'self' 'unsafe-inline'` kept the document's own files
  loading and blocked a request to another site, so blocking other sites is
  one header away.
- In an `http://localhost` page, an `iframe` of a `file://` URL stayed
  empty: Chrome refuses it with "Not allowed to load local resource". So the
  browser cannot show a file of its own machine in place, and no Host is
  special.

## Prior art

- [Jupyter Server](https://github.com/jupyter-server/jupyter_server/blob/main/jupyter_server/files/handlers.py)
  serves `/files/<path>` with `Content-Security-Policy: sandbox
  allow-scripts`, "to confine any Javascript to a unique origin so it can't
  interact with the notebook server", and refuses hidden files unless told
  otherwise. Demi takes both.
- [JupyterLab's HTML viewer](https://github.com/jupyterlab/jupyterlab/blob/main/packages/htmlviewer/src/widget.tsx)
  renders the file from a `blob:` URL with a `<base href>` pointing at its
  `/files/` URL, in a frame sandboxed with `allow-same-origin`. Scripts run
  only after "Trust HTML", which adds `allow-scripts` and so gives the page
  the server's origin. Demi runs scripts always, on an opaque origin, and
  resolves relative paths through a URL that is a path.
- [VS Code Live Preview](https://github.com/microsoft/vscode-livepreview/blob/main/src/server/httpServer.ts)
  serves the workspace from a local HTTP server, by default on port 3000,
  and shows it in a webview's `iframe`: a separate origin, where storage and
  root paths work. Demi has one origin for every Host.
- GitHub's `raw.githubusercontent.com` serves an HTML file as `text/plain`
  under `default-src 'none'; sandbox`, and shows no page.

## Alternatives

- **The file route with an `iframe`.** The page runs on Demi's origin with
  the session: any HTML file could act as the user.
- **`srcdoc` or a `blob:` URL with an injected `<base href>`, as
  JupyterLab.** The subresources go to the cookie route from an opaque
  origin, which sends no cookie, so they fail; with `allow-same-origin` they
  load, and the scripts get the session.
- **A separate origin, as Live Preview's local server or a preview domain.**
  Storage and root paths would work. A local server serves only the machine
  it runs on, a second domain is one more name and certificate for every
  instance to set up, and a page on a public HTTPS name reaching a local
  server meets Chrome's local network permission. Left for when a page needs
  it.
- **Open the file in the browser panel by `file://`.** Faithful, but it needs
  Chrome on the Host and draws the page as a video stream.

## Open questions

None.
