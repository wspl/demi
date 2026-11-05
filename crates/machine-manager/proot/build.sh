#!/bin/sh
# Builds the proot that runs Simple Isolation Clouds
# (crates/machine-manager/src/runtime/simple.rs), statically linked with
# talloc, from the sources release.json pins, and writes beside it the source
# archives it was built from, which a server release ships with it. It runs
# in the builder release.json names, on the architecture it builds for
# (README.md):
#
#   build.sh <output directory>
#
# writes <output directory>/proot and <output directory>/proot-sources/.
set -eu

output=$(mkdir -p "$1" && cd "$1" && pwd)
pins=$(cd "$(dirname "$0")" && pwd)/release.json
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
# clang, as proot's own tests build it: on arm64 it also builds the loader
# of 32-bit programs, which needs -m32. gawk generates the loader's tables.
apt-get install -y -qq --no-install-recommends build-essential clang gawk \
  python3 ca-certificates curl jq >/dev/null

work=$(mktemp -d)
mkdir -p "$output/proot-sources"

# Downloads a pinned source archive, checks it, and keeps it for the release.
fetch() {
  name=$1
  version=$(jq -r ".$name.version" "$pins")
  archive="$output/proot-sources/$name-$version.tar.gz"
  curl -fsSL -o "$archive" "$(jq -r ".$name.url" "$pins")"
  echo "$(jq -r ".$name.sha256" "$pins")  $archive" | sha256sum -c - >/dev/null
  tar -xzf "$archive" -C "$work"
  echo "$work/$name-$version"
}

talloc=$(fetch talloc)
proot=$(fetch proot)

# talloc alone, as a static library: its build system makes the
# configuration header, and the one source file is the library.
(
  cd "$talloc"
  ./configure --disable-python --disable-rpath --without-gettext >/dev/null
  cc -O2 -D_GNU_SOURCE -DHAVE_CONFIG_H -D__STDC_WANT_LIB_EXT1__=1 \
    -Ibin/default -Ilib/replace -I. -c talloc.c -o talloc.o
  ar rcs libtalloc.a talloc.o
)

make -s -C "$proot/src" proot CC=clang CFLAGS="-O2 -I$talloc" \
  LDFLAGS="-static -L$talloc -ltalloc" >/dev/null
cp "$proot/src/proot" "$output/proot"
strip "$output/proot"
chmod 755 "$output/proot"
rm -rf "$work"
