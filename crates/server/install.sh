#!/bin/sh
# Installs demi-server of the release a channel names, or of the version
# given, and goes on into its setup (docs/delivery/installation.md § The
# bootstrap):
#
#   curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --beta
#
# Demi has no stable release yet, so the script takes no default: `--beta`
# installs the newest release Demi publishes, `--canary` the newest build, a
# deploy's pre-releases included, and `--version <version>` that release.
# The other parameters go to `demi-server setup`. At a terminal, setup asks
# for what is missing; without one, as an agent runs it, it prints its
# guide.
set -eu
repository=wspl/demi
fail() {
  echo "install.sh: $1" >&2
  exit 1
}

# The highest version among the repository's releases tagged v<version>, as
# demi-server's fetch.rs picks it: other tags, such as the gVisor build's
# runsc-release-*, are no Demi release, and versions sort by their numbers,
# a version above its own pre-releases.
newest() {
  page=1
  while :; do
    tags=$(curl -fsSL -H 'Accept: application/vnd.github+json' \
      "https://api.github.com/repos/$repository/releases?per_page=100&page=$page" |
      grep -o '"tag_name": *"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
    [ -n "$tags" ] && printf '%s\n' "$tags"
    [ "$(printf '%s\n' "$tags" | grep -c .)" -lt 100 ] && break
    page=$((page + 1))
  done |
    sed -n 's/^v\([0-9]\{1,\}\.[0-9]\{1,\}\.[0-9]\{1,\}\(-[0-9A-Za-z.-]\{1,\}\)\{0,1\}\)$/\1/p' |
    awk '{ split($0, parts, "-"); split(parts[1], n, ".");
      print n[1], n[2], n[3], ($0 == parts[1] ? 1 : 0), $0 }' |
    sort -k1,1n -k2,2n -k3,3n -k4,4n -k5,5 | tail -n 1 | cut -d' ' -f5
}

# The tag of the repository's latest release, the newest Demi published,
# without its v; nothing while Demi has published none, which GitHub answers
# with 404. Any other failure, such as GitHub's limit on requests, stops the
# script with its reason.
latest() {
  answer=$(curl -sS -w '\n%{http_code}' -H 'Accept: application/vnd.github+json' \
    "https://api.github.com/repos/$repository/releases/latest") ||
    fail 'GitHub could not be asked for the newest beta release'
  status=$(printf '%s\n' "$answer" | tail -n 1)
  case "$status" in
    200) ;;
    404) return 0 ;;
    *) fail "GitHub answered $status when asked for the newest beta release" ;;
  esac
  printf '%s\n' "$answer" | grep -o '"tag_name": *"v[^"]*"' | sed 's/.*"v\([^"]*\)"$/\1/'
}

channel=
version=
# Whether --version was given, and whether its value is still to come.
versioned=
awaited=
for parameter do
  shift
  if [ -n "$awaited" ]; then
    version=$parameter
    awaited=
    continue
  fi
  case "$parameter" in
    --beta | --canary)
      [ -z "$channel" ] || fail 'choose one channel: --beta or --canary'
      channel=${parameter#--}
      ;;
    --version=*) version=${parameter#--version=} versioned=1 ;;
    --version) versioned=1 awaited=1 ;;
    *) set -- "$@" "$parameter" ;;
  esac
done
[ -z "$versioned" ] || [ -n "$version" ] || fail '--version names a version, such as --version 0.1.22'
[ -z "$channel" ] || [ -z "$versioned" ] || fail "--$channel installs the newest release of its channel: give it or --version, not both"
if [ -z "$channel" ] && [ -z "$versioned" ]; then
  fail 'Demi has no stable release yet: choose a channel.
  The releases Demi publishes:  curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --beta
  The newest build:             curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --canary'
fi
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
case "$channel" in
  beta)
    version=$(latest)
    [ -n "$version" ] || fail 'Demi has published no beta release yet: --canary installs the newest build'
    ;;
  canary)
    version=$(newest)
    [ -n "$version" ] || fail "$repository has no release tagged v<version>"
    ;;
esac
set -- "$@" --version "$version"
base="https://github.com/$repository/releases/download/v$version"
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
