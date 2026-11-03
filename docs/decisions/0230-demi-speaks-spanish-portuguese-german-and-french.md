---
status: accepted
---

# Demi speaks Spanish, Portuguese, German and French

## Problem

Lucía's browser prefers `es-MX`. Demi speaks English, Simplified Chinese,
Traditional Chinese, Japanese and Korean, so *System* resolves to English,
and Settings › General › Language offers her nothing closer. The same holds
for a reader of Portuguese, German or French, the most spoken languages after
English among the developers and teams Demi is for. Each reads every control,
refusal and email in a second language.

## Decision

Demi speaks four more languages, each complete, by the rules of
`localization.md`: every text Demi shows a person itself, in the web app,
every plugin's pages, the gallery, the page's notifications, emails and the
pages the backend renders. What a model reads and what people wrote stay as
they are.

| Tag | Named in the menu | Who it is for |
| --- | --- | --- |
| `de` | Deutsch | Germany, Austria, Switzerland |
| `es-419` | Español (Latinoamérica) | Latin America and the United States; Spain reads it too |
| `fr` | Français | France, Belgium, Switzerland, Canada |
| `pt-BR` | Português (Brasil) | Brazil; Portugal reads it too |

Settings › General › Language lists English first, then the languages in
Latin letters by their own names, then the others as today:

```
Language   System ▾
           ─────────────────────────
           System
           English
           Deutsch
           Español (Latinoamérica)
           Français
           Português (Brasil)
           简体中文
           繁體中文
           日本語
           한국어
```

### How System resolves

A browser tag of a language resolves to the one variant Demi speaks of it, in
the page (`i18n/languages.ts`) and in the backend's `Accept-Language`
(`backend-http`), by the same table:

| Browser's tag | Demi speaks |
| --- | --- |
| `de`, `de-AT`, `de-CH` | `de` |
| `es`, `es-MX`, `es-ES`, `es-US` | `es-419` |
| `fr`, `fr-CA`, `fr-BE` | `fr` |
| `pt`, `pt-BR`, `pt-PT` | `pt-BR` |

A Spanish reader in Spain and a Portuguese reader in Portugal are read in the
nearest variant Demi has, rather than in English.

### The design to build from

- `Language` and `LanguageChoice` in `web-api-protocol` gain `German`,
  `LatinAmericanSpanish`, `French` and `BrazilianPortuguese`, serialized as
  the tags above. An older page never sends them. A rollback to an older
  release cannot read the preferences of a user who chose one of them, as
  with any value an older release does not know.
- Every catalogue gains `de.json`, `es-419.json`, `fr.json` and `pt-BR.json`,
  and every backend catalogue its `.yml`. `Catalogue` already requires every
  language of `LANGUAGES`, so a key missing from one does not compile.
- A count picks its form by `Intl.PluralRules` of the page's language, in
  place of vue-i18n's English rule: French and Brazilian Portuguese say
  *0 fichier* and *0 arquivo* in the singular, English, German and Spanish
  *0 files* in the plural. Chinese, Japanese and Korean have no plural, so
  their texts with two forms keep the first for exactly 1, as *毎分* beside
  *{count} 分ごと*.
- German's long compound words wrap at a syllable: the page sets
  `hyphens: auto` under `:lang(de)`. The other three need no rule of their
  own, and all four keep the system's Latin font.
- A caption that holds a button keeps one line in every language. In
  Providers, the API Keys caption's Add button becomes a + icon whose tooltip
  says Add Provider, the dialog it opens, as the + in the sections of
  Finder's and Mail's sidebars: German's *API-Schlüssel* beside
  *Hinzufügen* needs 185 px where the list has 172.
- The gallery's Writing page gains a section per language: its rules, such as
  French's space before `:` and `?`, and its glossary, each term weighed
  against the products `localization.md` names.
  German, Spanish and Portuguese address the user as *du*, *tú* and *você*,
  as macOS and ChatGPT do; French as *vous*.
- The gallery's language menu lists the same nine languages.
- The README and the comparison (`docs/comparison.md`) gain a translation in
  each of the four languages, linked from every language bar of theirs.

## Spike

None.

## Prior art

- VS Code ships Spanish as one variant and Portuguese as Brazilian, beside
  German and French ([vscode-loc](https://github.com/microsoft/vscode-loc)).
- Claude's interface speaks Spanish (Latin America) and Portuguese (Brazil).
- Open WebUI and LibreChat keep a catalogue per variant, Portugal's and
  Brazil's Portuguese among them.

## Alternatives

- **`es` and `pt` with no region in the name.** ChatGPT and VS Code show
  *Español* and *Português*. The name hides which variant it is, so a
  reader in Spain or Portugal finds out only by reading it, and adding the
  other variant later renames the first.
- **Spain's Spanish, `es-ES`, and Portugal's Portuguese, `pt-PT`.** Far
  fewer readers, and the products of the same kind ship the Latin American
  and Brazilian variants first.
- **Every variant at once.** Twice the text to keep complete in every
  change, for readers who already read the nearer variant without stopping.

## Open questions

None.
