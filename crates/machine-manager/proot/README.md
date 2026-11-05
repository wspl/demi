# proot

`release.json` pins the proot that runs Simple Isolation Clouds
(`crates/machine-manager/src/runtime/simple.rs`):

- The image of the builder, an Ubuntu release by its digest.
- proot's source archive, a commit of Termux's fork of proot, and its
  SHA-256.
- The source archive of talloc, the one library proot links, its version
  and SHA-256.

## Why Termux's fork

Demi takes proot from Termux's fork, `github.com/termux/proot`, which is
maintained and tested on amd64 and arm64 Linux, rather than from
`github.com/proot-me/proot`. proot-me's releases and its main branch lack
fixes a Cloud needs:

- A process that forks before its first `brk(2)`, whose child then calls
  `brk(0)` before its own `execve(2)`, crashes proot. The runner does this
  when it starts a program in the background.
- On arm64, proot does not know `faccessat2(2)`, so `test -w` and `test -x`
  check the host's paths instead of the Cloud's, and package scripts fail.
- On both architectures, proot does not know `openat2(2)`, so a program that
  calls it opens the host's files.

## Building

proot publishes no builds, so Demi builds it, for each architecture on that
architecture, statically linked with talloc and glibc. Termux's fork uses
glibc's interfaces, so it does not build with musl. The server release job of
`.github/workflows/build.yml` runs `build.sh` in the builder:

```sh
builder=$(jq -r .builder crates/machine-manager/proot/release.json)
docker run --rm -v "$PWD:/work" -w /work "$builder" \
  sh crates/machine-manager/proot/build.sh native-target/aarch64-unknown-linux-musl/release
```

The server release holds the program as `bin/proot`, and the two source
archives it was built from in `sources/`, which the release also publishes as
assets: proot is under the GNU GPL 2.0 or later and talloc under the GNU LGPL
3.0 or later, which ask for the source to accompany the program. Their
notices are `LICENSE-proot` and `LICENSE-talloc`.

## Adopting a new commit

A new pin passes these on both architectures, in an Ubuntu tree of the Cloud
image, as an ordinary user:

- `proot -0 -r <tree> /usr/bin/apt-get install -y python3` succeeds, its
  package scripts included.
- `sudo -n true` under `proot -0` succeeds.
- Ending the first process of `proot --kill-on-exit` ends the processes it
  started, those that called `setsid` included, and proot.
- A Cloud runs a command in the product, which starts programs in the
  background.
