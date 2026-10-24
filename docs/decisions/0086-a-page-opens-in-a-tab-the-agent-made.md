---
status: proposed
---

# A page opens in a tab the agent made

## Problem

An agent edits a local page, `report.html`, and checks each edit the same
way:

```shell
demi browser open file:///Users/alex/report.html && \
  demi browser screenshot t1 | demi file view
```

`demi browser open` makes a new tab each time. After twenty edits the
user's work panel holds twenty tabs of the same page, and the browser runs
twenty copies of it until the conversation is released. `demi browser goto`
would have loaded the page again in the first tab, but the agent never used
it: in the agent browser tools it knows, `open` is the verb that loads a page
in a tab, and making a tab is a command of its own (Prior art).

## Decision

**Making a tab and opening a page are two commands, and every command that
acts on a tab names it:**

```shell
$ demi browser tab new
t1
$ demi browser open t1 file:///Users/alex/report.html
$ demi browser screenshot t1 | demi file view
# After the next edit, the same tab:
$ demi browser open t1 file:///Users/alex/report.html
```

The model reads these leaves:

| Leaf | What it does |
|---|---|
| `tab new` | Makes an empty tab on this Host, starting the conversation's browser when needed, and prints its ID. The tab joins the user's work panel without taking the user's view. |
| `tab list` | Lists the conversation's live tabs without starting a browser, as `tabs` did. |
| `tab close <tab>` | Closes a tab. Closing the last one ends the browser; the next `tab new` starts it again. |
| `open <tab> <url>` | Opens a URL in the tab, in place of its page, as `goto` did; `--load` and `--timeout` as before, and `--show` also shows the tab, as `show` does. |

`demi browser open <url>`, `goto`, `tabs` and `close` are gone, with no
alias. There is no current tab: one browser serves all of a conversation's
agents and their parallel commands, so a command without a tab would act
on whichever tab another command made current.

The design to build from:

* The `demi.browser` package's operations change with the leaves, one leaf
  per operation as before (`crates/plugin-browser/plugin/src/browser.rs`):

  ```rust
  "tab.new" => TabNewInput, TabNewResult;       // new
  "tab.list" => TabListInput, TabListResult;    // was "tabs", TabsInput
  "tab.close" => TabCloseInput, TabCloseResult; // was "close", CloseInput
  "open" => OpenInput, NavigationResult;        // was "goto", GotoInput

  /// `tab.new`: makes an empty tab, starting the browser when it does not
  /// run. A cold start launches Chrome, so its default deadline is the
  /// longest, as the old `open`'s was.
  pub struct TabNewInput {
      pub timeout: Option<u64>,
  }

  /// What `tab.new` answers: the new tab, with its viewport when the page
  /// reported it in time.
  pub struct TabNewResult {
      pub tab: TabId,
      pub viewport: Option<BrowserViewport>,
  }

  /// `open`: opens a URL in a tab, as `goto` did, and shows the tab when
  /// `show` is set.
  pub struct OpenInput {
      pub tab: TabId,
      pub url: String,
      pub load: Option<Load>,
      pub show: Option<bool>,
      pub timeout: Option<u64>,
  }
  ```

* The work panel's page methods keep their names and inputs. `bind`, which
  opens a browser tab for a panel tab whose browser lost it, calls
  `tab.new` and then `open`; the panel binds the new tab to the panel tab
  before the page loads, so no second panel tab is added for it, as today.
  When `open` fails, the panel tab keeps the address its user asked for,
  not the blank page its new tab still shows, so Retry opens that address.
  `navigate` calls `open`.
* `tab new` prints the tab's ID alone, so a script takes it as
  `T=$(demi browser tab new)`; `--json` prints the tab and its viewport.
* Nothing is stored differently: tab IDs, the panel's tabs and the
  browser's lifetime are unchanged.
* An older runner meets no other operations than its backend's: the
  backend's startup catalog pins the browser package release its leaves
  name, and a runner fetches that release. An agent that still runs
  `demi browser goto t1 …` gets the command line's usual error, `unrecognized
  subcommand 'goto'`, with the group's usage line.

## Spike

None.

## Prior art

* Playwright makes a page with
  [`browserContext.newPage()`](https://playwright.dev/docs/api/class-browsercontext#browser-context-new-page)
  and loads it with
  [`page.goto(url)`](https://playwright.dev/docs/api/class-page#page-goto);
  the Chrome DevTools Protocol has
  [`Target.createTarget`](https://chromedevtools.github.io/devtools-protocol/tot/Target/#method-createTarget)
  and
  [`Page.navigate`](https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-navigate),
  and WebDriver BiDi has
  [`browsingContext.create`](https://w3c.github.io/webdriver-bidi/#command-browsingContext-create)
  and
  [`browsingContext.navigate`](https://w3c.github.io/webdriver-bidi/#command-browsingContext-navigate).
  None of them has a current tab.
* [agent-browser](https://github.com/vercel-labs/agent-browser) loads a page
  with `open <url>` (aliases `goto`, `navigate`) and makes and closes tabs
  with `tab new` and `tab close`. It acts on an active tab, which this
  design does not have.
* [Playwright MCP](https://github.com/microsoft/playwright-mcp) loads a page
  in the current tab with `browser_navigate` and makes a tab only through
  `browser_tabs`. It too has a current tab.

## Alternatives

* **Keep `open` and `goto`, and say in `open`'s help that tabs stay.** The
  agent had `goto` and its help; a sentence weighs less than a verb that
  means something else in every tool the model knows.
* **A current tab that a command without a tab acts on**, as agent-browser
  and Playwright MCP have. One conversation's agents and parallel commands
  share the browser, so what a bare command acts on would depend on what
  another one did last.
* **`tab new <url>`.** Making a tab with its page in one step is what `open`
  did, and checking a page again would again make a tab.
* **`tab ls`.** Demi's other groups list with `list`, as `demi host list`
  and `demi agent list` do.
* **A limit on tabs, or freeing tabs no one looked at for a while.** With
  tabs reused there is nothing to free; either would add a count or an idle
  timer that the browser's lifetime deliberately does without.

## Open questions

None.
