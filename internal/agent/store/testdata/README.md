# Image fixture

`gopher.webp` is `testdata/gopher-doc.1bpp.lossless.webp` from
`golang.org/x/image` v0.46.0. Its BSD license is in `x-image-LICENSE`.
It covers kept-original WebP handling; the standard library has no WebP
encoder. The tests generate their PNG, JPEG and GIF images in memory.
