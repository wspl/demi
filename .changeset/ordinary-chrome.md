---
"@demicodes/browser-protocol": minor
"@demicodes/coding-agent": patch
---

The conversation browser is an ordinary Chrome to the pages it opens: no automation marker (`navigator.webdriver` is false), the Host platform's standard user agent without the headless token, visible scrollbars, a mouse with hover on Linux too, windows that hold their viewports, and the user's time zone and languages from the command context. A tab's viewport now reports its `devicePixelRatio` and `mode` (`web` or `custom`); `viewport set` takes `--scale`, `viewport set` and `reset` answer `{ viewport }`, and every screenshot is in CSS pixels whatever the ratio. A tab can be created by the `user`. The environment always loads the live view's capture extension.
