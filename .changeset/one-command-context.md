---
"@demicodes/web": minor
---

The page reports the browser's time zone and languages as the user's `locale`
preference, and declared commands receive it in their command context; until
the page has reported it, commands get `UTC` and `en-US`.
