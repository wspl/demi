/**
 * A short release's third-party notices, in the layout `xtask notices`
 * writes: each license text once, under the packages it covers.
 */
export const DEMO_NOTICES = `Third-party notices

Demi includes the third-party software below. Each notice names the
packages under one license text, then gives the text.


Rust crates
===========

MIT
---

tokio 1.48.0
tokio-util 0.7.17

Copyright (c) Tokio Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

MPL-2.0
-------

webauthn-rs 0.5.5, source: https://github.com/kanidm/webauthn-rs

Mozilla Public License Version 2.0
==================================


npm packages
============

MIT
---

vue 3.5.21

The MIT License (MIT)

Copyright (c) 2018-present, Yuxi (Evan) You and Vue contributors
`

/** What the About specimen's stand-in server answers for the notices. */
export type NoticesAnswer = 'notices' | 'none' | 'failure'
