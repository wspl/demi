# Localization

Demi speaks English and Simplified Chinese. A user who picks 简体中文 in
Settings › General sees every control, message and email in Chinese at once,
without a reload; what a model reads and what people wrote stay as they are.
This page says what is translated, where each text lives, how text that
comes from the backend is worded, and how the language is chosen. How each
language is written, its glossary included, is the gallery's Writing page.

## What is translated

Every text Demi itself shows a person: the web app, every plugin's pages,
the gallery, the notifications the page shows, emails, and the pages the
backend renders itself, such as the sign-in callback.

What is never translated:

- What a model reads: the agent's guide and system prompt, tool results,
  `demi` command help and output. It stays English in every language, so
  the agent's behavior does not depend on the user's.
- What people and models wrote: messages, replies, summaries and the titles
  a model gives conversations, which follow the language the user writes in
  ([Product](product.md)).
- Names: files, paths, commands, models, vendors, branches, accounts.
- A vendor's or a system's own words quoted as they came, such as a
  provider's response, git's output or an operating system's error. They
  show under their disclosure, below a headline Demi words in the user's
  language from the failure's code.

## Choosing the language

Settings › General › Language offers *System*, *English* and *简体中文*,
each named in its own language. The choice is the `language` preference
([Web API](web-api.md)), so every page of the user and the backend's emails
follow it; *System* is the default and takes the first of the browser's
languages that Demi speaks, any `zh` tag counting as 简体中文, else English.
A change applies at once in every open page, as other preferences do, and
the document's `lang` follows it.

## Where the text lives

Each package owns its text, as it owns its components: `web-ui`, `web`,
`web-gallery` and each plugin package keep a catalogue per language,
`src/locales/en.json` and `src/locales/zh-Hans.json`, English the source.
Keys name the place and the meaning, `settings.general.language`, never the
English words, so rewording English keeps the key.

The page uses `vue-i18n` (the Composition API). The app, `web` or the
gallery, creates the one instance with the language chosen above; `web-ui`'s
components read their own catalogue through it, and a plugin reads its own
through `@demicodes/plugin-sdk`, under its plugin's id, so a plugin brings
its text with it and the shell knows none of it
([Plugin pages](../architecture/plugin-pages.md)). Each catalogue is typed by
its English file, so a key missing from a language or misspelled at its use
does not compile. A count uses the catalogue's plural forms; Chinese has
one.

Numbers, sizes, dates, times, relative ages and lists are formatted with
`Intl` in the chosen language by the existing formatters, never by hand in
a component: *2h* and *2 小时*, *Oct 15* and *10月15日*.

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
  *System* in the first of the languages their browser last reported (the
  `locale` preference); a page in the request's `Accept-Language`.
