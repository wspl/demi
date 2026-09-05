# Demi Gallery

Live component catalog for `@demicodes/web-ui`. Style axes remap token values
(`surface`, `fg`, `line`, shadow, radius, density, accent) so paradigms can be
compared against the catalog. The product keeps a single light/dark pair;
this package does not ship themes into runtime.

```bash
bun run web:gallery
```

Opens `http://127.0.0.1:18933`.

## Color review

`/color-review` lists the colour problems found under the product appearance, each with the
shipped component next to the same component under the proposed fix (`src/color-review/`).
Decisions made on the page are written by the dev server to
`packages/web-gallery/.color-review/decisions.json` (git-ignored) so they can be read back.
