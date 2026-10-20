# Localization

Demi speaks English, Simplified Chinese, Traditional Chinese, Japanese and
Korean, each complete. A user who picks 简体中文 in Settings › General sees every control, message and email in Chinese at once,
without a reload; what a model reads and what people wrote stay as they are.
This page says what is translated and how a word is chosen. How each
language is written, its glossary included, is the gallery's Writing page.
How the language is chosen, where each text lives and how text from the backend
is worded is in the code: `packages/web-ui/src/i18n/`, `packages/web-ui/src/infra/errors.ts`
and `crates/web-api-protocol/src/error.rs`.

English is decided first, and the other languages follow it: each says
what the English says, in the words its users would find most natural for
the thing in front of them, never a literal rendering, and no word chosen
for another language changes the English. No single source decides a text. Each term and each phrasing is chosen by comparing many
products that speak the language well and say the same thing: AI apps of
the same kind (Codex, ChatGPT, Claude, Cursor), the system's own apps
(macOS and iOS: Mail, Finder, System Settings), and for developer
concepts, such as commit, branch, terminal or diff, the tools developers
read every day (VS Code, Git clients). Where they agree, take theirs; where
they differ, take the one a user of that region reads without stopping,
and prefer everyday words over a vendor's house terminology, such as
归档 over 存档 and 账户 over 帐户. A word a product uses for a different
thing that shares an English name does not count: a model vendor is
供应商, not an editor's 提供程序. The glossary records each choice with
the products it was weighed against, and a better choice found later
replaces it everywhere.

## What is translated

Every text Demi itself shows a person: the web app, every plugin's pages,
the gallery's own controls and its specimens, the notifications the page shows, emails, and the pages the
backend renders itself, such as the sign-in callback.

What is never translated:

- What a model reads: the agent's guide and system prompt, tool results,
  `demi` command help and output. It stays English in every language, so
  the agent's behavior does not depend on the user's.
- What people and models wrote: messages, replies, summaries and the titles
  a model gives conversations, which follow the language the user writes in
  (`crates/agent-server/src/title.rs`).
- Names: files, paths, commands, models, vendors, branches, accounts, and
  the product's own. In a sentence it is *Demi*, *Demi needs permission*,
  *Add Demi to Your Home Screen*, and so are the license's names, the
  *Demi License* and *The Demi Authors*. Where the name stands alone as the
  product's mark, it is the wordmark *demi*, in lowercase in every language,
  as Amazon's and Intel's logos are while their sentences say Amazon and
  Intel: the sidebar's header, the end of the page's title, *整理季度报告
  · demi*, the installed app's name under its icon and in the Dock, the
  name beside the logo on the sign-in page, a notification's title when it
  has none of its own, the name a passkey shows for the site, and the name of the certificate that
  signs the Mac runner, *demi Code Signing*.
- The gallery's notes, which document a section for the people building
  Demi, and the content of its fixtures, such as a specimen conversation's
  messages, which stand for what users write.
- A vendor's or a system's own words quoted as they came, such as a
  provider's response, git's output or an operating system's error. They
  show under their disclosure, below a headline Demi words in the user's
  language from the failure's code.
