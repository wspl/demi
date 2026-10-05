#!/bin/sh
# Installs demi-server of Demi @VERSION@ and goes on into its setup
# (docs/delivery/installation.md § The bootstrap):
#
#   curl -fsSL https://github.com/wspl/demi/releases/latest/download/install.sh | sudo bash
#
# Parameters after `bash -s --` go to `demi-server setup`. At a terminal,
# setup asks for what is missing; without one, as an agent runs it, it prints
# its guide.
set -eu
version=@VERSION@
base="https://github.com/wspl/demi/releases/download/v$version"
fail() {
  echo "install.sh: $1" >&2
  exit 1
}
[ "$(uname -s)" = Linux ] || fail 'Demi servers run on Linux'
[ "$(id -u)" = 0 ] || fail 'run as root: curl ... | sudo bash'
case "$(uname -m)" in
  x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
  aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
  *) fail "Demi runs on no $(uname -m) server" ;;
esac
if [ -e /opt/demi/current ] && [ ! -e /opt/demi/server/setup ]; then
  echo 'This machine is a Demi server already: `demi-server upgrade` moves it to a later release.'
  exit 0
fi
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
curl -fsSL "$base/SHA256SUMS" -o "$stage/SHA256SUMS"
curl -fsSL "$base/demi-server-$target" -o "$stage/demi-server-$target"
(cd "$stage" && grep " demi-server-$target\$" SHA256SUMS | sha256sum -c - > /dev/null) ||
  fail "demi-server-$target does not match its SHA-256 in SHA256SUMS"
install -m 0755 "$stage/demi-server-$target" /usr/local/bin/demi-server
# This script's own input is the script; setup asks at the session's
# terminal when there is one.
if (: < /dev/tty) 2> /dev/null; then
  exec /usr/local/bin/demi-server setup "$@" < /dev/tty
fi
exec /usr/local/bin/demi-server setup "$@"
