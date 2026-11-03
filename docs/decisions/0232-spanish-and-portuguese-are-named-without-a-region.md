---
status: accepted
---

# Spanish and Portuguese are named without a region

Supersedes: 0230 (Demi speaks Spanish, Portuguese, German and French), the
names `es-419` and `pt-BR` have in the language menu.

## Problem

Settings › General › Language names Latin American Spanish *Español
(Latinoamérica)* and Brazilian Portuguese *Português (Brasil)*. Demi speaks
one Spanish and one Portuguese, so the region tells each from nothing, and a
reader in Spain or Portugal, who reads them too, takes the menu for a choice
it does not offer.

## Decision

The language menu, the gallery's language menu and its Writing page, and the
language bars of every README and comparison name `es-419` *Español* and
`pt-BR` *Português*:

```
Language   System ▾
           ─────────────────────────
           English
           Deutsch
           Español
           Français
           Português
           简体中文
```

The tags stay `es-419` and `pt-BR`: they say which Spanish and which
Portuguese the catalogues are written in, and stored preferences keep them.

## Spike

None.

## Prior art

macOS and iOS add a region to a language's name where they offer more than
one form of it, such as *Português (Brasil)* beside *Português (Portugal)*.
Demi offers one of each, so their names need no region.

## Alternatives

- Keep the regions: say which Spanish and Portuguese they are, at the cost of
  a choice the menu does not have.

## Open questions

None.
