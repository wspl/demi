# Localization

Demi speaks English, Simplified Chinese, Traditional Chinese, Japanese and
Korean, each complete. A user who picks 简体中文 in Settings › General sees every control, message and email in Chinese at once,
without a reload; what a model reads and what people wrote stay as they are.
This page says what is translated, where each text lives, how text that
comes from the backend is worded, and how the language is chosen. How each
language is written, its glossary included, is the gallery's Writing page.

English is decided first, and the other languages follow it: each says
what the English says, in the words its users would find most natural for
the thing in front of them, never a literal rendering, and no word chosen
for another language changes the English. For example, the Workflow menu's
*No Subagents* is 不使用子智能体, not an English reworded to match a
Chinese 关闭子智能体. No single source decides a text. Each term and each phrasing is chosen by comparing many
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
  ([Product](product.md)).
- Names: files, paths, commands, models, vendors, branches, accounts.
- The gallery's notes, which document a section for the people building
  Demi, and the content of its fixtures, such as a specimen conversation's
  messages, which stand for what users write.
- A vendor's or a system's own words quoted as they came, such as a
  provider's response, git's output or an operating system's error. They
  show under their disclosure, below a headline Demi words in the user's
  language from the failure's code.

## Choosing the language

Settings › General › Language offers *System*, *English*, *简体中文*,
*繁體中文*, *日本語* and *한국어*, each named in its own language. The choice is the `language` preference
([Web API](web-api.md)), so every page of the user and the backend's emails
follow it; *System* is the default and takes the first of the browser's
languages that Demi speaks, else English: `zh-Hant`, `zh-TW`, `zh-HK` and
`zh-MO` count as 繁體中文 and any other `zh` tag as 简体中文.
The page alone resolves *System*, and reports the language it resolved
with the browser's other facts in the `locale` preference, so the backend
reads that instead of resolving it again. The page keeps the language it
last showed in the browser's storage and starts in it, so a user whose
choice differs from the browser's sees no other language before the
preferences arrive. A change applies at once in every open page, as other
preferences do, and
the document's `lang` follows it, so each language's own font draws its
characters: one Han character looks different in Chinese, Japanese and
Korean, and the font stack names each language's system font (PingFang SC,
PingFang TC, Hiragino Sans, Apple SD Gothic Neo, then Noto Sans CJK) under
its `:lang()`.

## Where the text lives

Each package owns its text, as it owns its components: `web-ui`, `web`,
`web-gallery` and each plugin package keep a catalogue per language,
`src/locales/en.json`, `zh-Hans.json`, `zh-Hant.json`, `ja.json` and
`ko.json`, English the source. A large package splits its catalogue by
folder, `src/agent/locales/`, each folder's text under its own scope, so
the text of one area is found and changed beside its components.
Keys name the place and the meaning, `settings.general.language`, never the
English words, so rewording English keeps the key.

The page uses `vue-i18n` (the Composition API). The app, `web` or the
gallery, creates the one instance with the language chosen above; `web-ui`'s
components read their own catalogue through it, and a plugin reads its own
through `@demicodes/plugin-sdk`, under its plugin's id, so a plugin brings
its text with it and the shell knows none of it
([Plugin pages](../architecture/plugin-pages.md)). Each catalogue is typed by
its English file, so a key missing from a language or misspelled at its use
does not compile, so a new text is written in every language with the
change that adds it. A count uses the catalogue's plural forms; Chinese,
Japanese and Korean have one.

Numbers, sizes, dates, times, relative ages and lists are formatted with
`Intl` in the chosen language by the existing formatters, never by hand in
a component: *2h*, *2 小时* and *2 時間*, *Oct 15* and *10月15日*.

## Text from the backend

The backend sends codes and values, and the page words them:

- **A refused request** answers `{ code, message, params?, requestId }`. The
  page shows its catalogue's text for `code`, filled from `params`, the
  values the text needs, such as `{ limit: 1000 }`. `message` stays English,
  for the server log, API clients and the CLI, and the page shows it only
  for a code its catalogue does not have, which a newer backend may send.
- **A failed turn** shows a headline worded from its failure code and its
  [failure facts](../backend/backend.md#failure-facts), such as the reset
  time, and the vendor's own text below it, untranslated.
- **Transcript notices**, such as an account switch, a compaction or a
  command's end, are blocks holding values, worded by the page; a block
  never stores a sentence for the page to show.
- **Emails and the pages the backend renders** are worded by the backend
  from its own catalogue, one per language beside the crate that sends
  them, with `rust-i18n`: an email in the recipient's `language`, or with
  *System* in the language their page last resolved (the `locale`
  preference); a page in the request's `Accept-Language`.
