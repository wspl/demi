---
status: proposed
---

# An error of the page can show the ID that finds its line in the log

## Problem

A user opens a conversation and the transcript stays empty. A handler threw
as the page rendered it, and nothing caught the error: it reached the browser's
console and nowhere else. The log has no line for it and the page shows
nothing, so the user has no ID to hand to whoever runs the server. About 25
calls of `reportError` log a defect the same way: the line is in the log, but
the user never learns its ID.

## Decision

The page reports every JavaScript error that is a defect, the uncaught ones
included, and a setting decides whether the page shows it.

- **Logged always.** An error thrown in a handler or a timer, a rejected promise
  that no one waits for and an error in a Vue render, watcher or lifecycle hook
  is written to the log as an `error.reported` line, like any defect the page
  finds (`packages/web-ui/src/infra/errors.ts`).
- **Shown on request.** Settings › Developer, a page of its own, has *Show
  unexpected errors*, off by default. With it on, each defect shows a toast. The
  toast of an error nothing caught, or one a plugin's page reports as its own, is
  titled *Something Went Wrong*; one a caller reports keeps the caller's title.
  Its body is the error's own message on one line, ending in an ellipsis when it
  does not fit, and its Request ID line has a Copy button; the ID finds the line
  in the log. With it off, the page shows what it shows without this decision:
  the toasts its callers mark user-visible, with their IDs. Until the preferences
  arrive nothing more is shown. It is an account preference
  (`show_unexpected_errors`), as the language and the theme are.
- **A rejection that names a request.** An unhandled rejection whose error carries
  a request's ID is a defect of the page too: it is logged, with that request in
  its detail, and its toast shows that request's ID, not the line's.
- **Not a defect.** A cancelled operation, a failure of the environment (no
  network, a part of the page that did not download), the browser's notices that no
  code threw (ResizeObserver's), an extension's script and the browser's *Script
  error.* for a script of another origin are neither logged as a defect nor
  shown.
- **Once on screen.** Every occurrence writes its line. While a defect's toast is
  shown, a repeat of it (its title, message and where it failed) shows nothing
  more.

```text
 uncaught error ──► reportError ──► log line ──► Request ID: req_…  [Copy]
                         │                            ▲
                         └── setting on? ── toast ────┘
```

The page installs the handlers in `infra/uncaught.ts` (`window` `error` and
`unhandledrejection`, and the app's `errorHandler`), in the product and in the
gallery. A toast's title is the catalogue's, in every language; its body is the
error's own message as the code wrote it, in English.

Page content stays out of the log: a thrown value that is not an `Error` is logged
by its kind, and a `SyntaxError` loses the quoted text it could not read. The
message of any other error goes as thrown, so code that throws keeps page content
out of it, as the log's rule already requires.

## Spike

None.

## Prior art

Sentry's browser SDK installs the same three hooks (`window.onerror`,
`unhandledrejection` and the framework's error handler) and ignores the same
benign events (ResizeObserver's, extensions', *Script error.*)
([Sentry JavaScript: Global handlers](https://docs.sentry.io/platforms/javascript/configuration/integrations/default/)).
It reports to a service and drops consecutive identical events (its Dedupe
integration); Demi writes to its own log, whose retention bounds it, and keeps
every occurrence, so a defect's count and times are there. Safari's *Show features
for web developers* is a setting that is off by default for what only a developer
needs.

## Alternatives

- **Always show.** A page whose render fails every frame would stack toasts for a
  user who cannot act on them; most defects are for whoever runs the server.
- **A setting of the browser.** It would work signed out, but the choice would
  not follow the user to another browser, as neither the language nor the theme does.

## Open questions

None.
