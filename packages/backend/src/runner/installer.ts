import { createHash } from 'node:crypto'
import type { RunnerRelease } from '@demicodes/runner-protocol/release'

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`
}

export function shellInstaller(
  backend: string,
  release: RunnerRelease
): string {
  const url = new URL(backend)
  const http = url.protocol === 'https:' || url.protocol === 'http:'
  if (!http || url.username || url.password || url.search || url.hash) {
    throw new Error('invalid installation backend URL')
  }
  const registration = createHash('sha256').update(url.toString()).digest('hex')
  const cases = Object.entries(release.targets).map(([name, hashes]) => `
  ${name})
    runner_hash=${quote(hashes.runner)}
    client_hash=${quote(hashes.client)}
    ;;`).join('\n')
  return `#!/bin/sh
set -eu
backend=${quote(url.toString())}
release=${quote(release.release)}
registration=${quote(registration)}
base=${quote(url.origin)}
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) target=macos-arm64 ;;
  Darwin:x86_64) target=macos-x64 ;;
  Linux:aarch64|Linux:arm64) target=linux-arm64 ;;
  Linux:x86_64) target=linux-x64 ;;
  *)
    echo 'Unsupported runner platform' >&2
    exit 1
    ;;
esac
case "$target" in
${cases}
  *)
    echo 'This backend has no runner release for this platform' >&2
    exit 1
    ;;
esac
instance=\${DEMI_INSTALLATION_ID:-$registration}
case "$instance" in
  ''|*[!a-zA-Z0-9_-]*)
    echo 'Invalid installation ID' >&2
    exit 1
    ;;
esac
state="$HOME/.demi/instances/$instance"
bin="$state/releases/$release"
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
stage=$(mktemp -d "$state/releases/.download-XXXXXX")
cleanup() {
  rm -r "$stage"
  rmdir "$state/install.lock"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
verify() {
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$1")
  else
    actual=$(shasum -a 256 "$1")
  fi
  actual=\${actual%% *}
  if [ "$actual" != "$2" ]; then
    echo 'Runner download checksum mismatch' >&2
    exit 1
  fi
}
curl -fSL "$base/runner-artifacts/$release/$target/demi-runner" -o "$stage/demi-runner"
curl -fSL "$base/runner-artifacts/$release/$target/demi" -o "$stage/demi"
verify "$stage/demi-runner" "$runner_hash"
verify "$stage/demi" "$client_hash"
chmod 755 "$stage/demi-runner" "$stage/demi"
if [ -d "$bin" ]; then
  verify "$bin/demi-runner" "$runner_hash"
  verify "$bin/demi" "$client_hash"
else
  mv "$stage" "$bin"
  mkdir "$stage"
fi
if [ -x "$state/run" ]; then
  current=$("$state/run" status 2>/dev/null || true)
  if [ "$current" = "running $release" ]; then
    echo "Runner already running: $state"
    exit 0
  fi
  if [ -n "$current" ]; then
    echo 'Waiting for existing jobs before upgrading this runner…'
    "$state/run" drain
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
exec "$state/releases/$release/demi-runner" "\${1:-run}" --backend "$backend"
LAUNCHER
printf '%s\\n' "$backend" > "$state/backend-url"
printf '%s\\n' "$release" > "$state/release-id"
chmod 755 "$state/run.next"
mv "$state/run.next" "$state/run"
nohup "$state/run" > "$state/runner.log" 2>&1 < /dev/null &
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
printf 'Runner installed for %s\\nState and pairing log: %s\\n' "$backend" "$state/runner.log"
cat "$state/runner.log"
`
}
