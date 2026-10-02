#!/bin/sh
set -eu
backend=@BACKEND@
release=@RELEASE@
registration=@REGISTRATION@
base=@BASE@
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) target=aarch64-apple-darwin ;;
  Darwin:x86_64) target=x86_64-apple-darwin ;;
  Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-musl ;;
  Linux:x86_64) target=x86_64-unknown-linux-musl ;;
  *)
    echo 'Unsupported runner platform' >&2
    exit 1
    ;;
esac
case "$target" in
@CASES@
  *)
    echo 'This backend has no runner release for this platform' >&2
    exit 1
    ;;
esac
instance=${DEMI_INSTALLATION_ID:-$registration}
case "$instance" in
  ''|*[!a-zA-Z0-9_-]*)
    echo 'Invalid installation ID' >&2
    exit 1
    ;;
esac
state="$HOME/.demi/instances/$instance"
bin="$state/releases/$release"
# The installation's files are the user's alone; the runner works with the
# mask of the shell this installer runs in (runner.md § Builtins that act on
# a process).
user_umask=$(umask)
umask 077
mkdir -p "$state/releases"
if [ -f "$state/backend-url" ]; then
  IFS= read -r existing_backend < "$state/backend-url"
  if [ "$existing_backend" != "$backend" ]; then
    echo 'Installation belongs to another backend' >&2
    exit 1
  fi
fi
if ! mkdir "$state/install.lock" 2>/dev/null; then
  echo 'Another installer is active for this installation' >&2
  exit 1
fi
stage=
cleanup() {
  if [ -n "$stage" ]; then
    rm -r "$stage"
  fi
  rmdir "$state/install.lock"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
stage=$(mktemp -d "$state/releases/.download-XXXXXX")
verify() {
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$1")
  else
    actual=$(shasum -a 256 "$1")
  fi
  actual=${actual%% *}
  if [ "$actual" != "$2" ]; then
    echo 'Runner download checksum mismatch' >&2
    exit 1
  fi
}
curl -fSL "$base/runner-artifacts/$release/$target/demi-runner" -o "$stage/demi-runner"
verify "$stage/demi-runner" "$runner_hash"
chmod 755 "$stage/demi-runner"
if [ -d "$bin" ]; then
  verify "$bin/demi-runner" "$runner_hash"
else
  mv "$stage" "$bin"
  mkdir "$stage"
fi
if DEMI_HOME="$state" DEMI_RELEASE_ID="$release" "$bin/demi-runner" status --backend "$backend" >/dev/null 2>&1; then
  echo "Runner already running: $state"
  exit 0
else
  status=$?
  if [ "$status" -eq 3 ]; then
    echo 'Waiting for existing jobs before upgrading this runner…'
    DEMI_HOME="$state" "$bin/demi-runner" drain --backend "$backend"
  fi
fi
# Pass values as quoted arguments; never interpolate a backend into executable shell code.
cat > "$state/run.next" <<'LAUNCHER'
#!/bin/sh
set -eu
state=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
IFS= read -r backend < "$state/backend-url"
IFS= read -r release < "$state/release-id"
export DEMI_HOME="$state" DEMI_RELEASE_ID="$release"
exec "$state/releases/$release/demi-runner" "${1:-run}" --backend "$backend"
LAUNCHER
printf '%s\n' "$backend" > "$state/backend-url"
printf '%s\n' "$release" > "$state/release-id"
chmod 755 "$state/run.next"
mv "$state/run.next" "$state/run"
# The log is made here, under 077: it holds the pairing code.
nohup sh -c 'umask "$1" && exec "$2"' sh "$user_umask" "$state/run" > "$state/runner.log" 2>&1 < /dev/null &
pid=$!
tries=0
until "$state/run" status >/dev/null 2>&1; do
  tries=$((tries + 1))
  if ! kill -0 "$pid" 2>/dev/null || [ "$tries" -ge 100 ]; then
    cat "$state/runner.log" >&2
    exit 1
  fi
  sleep 0.1
done
printf 'Runner installed for %s\nState and pairing log: %s\n' "$backend" "$state/runner.log"
cat "$state/runner.log"
