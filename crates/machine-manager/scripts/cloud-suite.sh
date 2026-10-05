#!/usr/bin/env bash
# Runs the Cloud suite (scenarios.md § Cloud suite) on a Linux machine, as
# root, against a manager of its own, never an installed one, with its
# resource limits off, in a stand-in execution host. The stand-in is the init
# of a throwaway PID and mount namespace with its own /run and an empty,
# read-only cgroup root, so nothing in it can create a cgroup and the
# machine's cgroup hierarchies stay untouched; it shares the machine's network
# namespace, so the Clouds reach the backend.
#
# After the run, also after a failure or an interruption, the script stops the
# manager, which saves every Cloud, ends the stand-in, deletes the manager's
# nftables table, restores IP forwarding, and removes the state directory. It
# then compares what a run can leave behind with its state before the run, and
# fails when anything remains.
#
# By default the manager and demi-server come from the workspace's build and
# the suite runs with cargo test; build the Cargo selection first, so they
# are in target/debug:
#   cargo build --workspace --all-targets --features demi-runner/test-fixtures
#
# --programs names a directory of programs built for this machine elsewhere,
# as a Mac builds them for its Lima VM (mac-development.md § Machine manager
# in Lima): demi-machine-manager, demi-server, and the backend's scenario
# test executable under the name backend-scenarios, which the script runs
# in place of cargo test, with the same arguments.
#
# --release names the server release root whose image/ the manager imports
# and whose commands/, the command packages the image embeds, the backend
# publishes.
#
# Usage: sudo bash crates/machine-manager/scripts/cloud-suite.sh --release DIR
#          --work DIR [--programs DIR] [--runsc PATH] [--dns ADDRESSES]
#          [--address ADDRESS] [-- TEST-ARGUMENTS...]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repository="$(cd "$here/../../.." && pwd)"
release=""
work=""
programs=""
runsc=""
dns=""
address=""
usage() {
  echo 'usage: cloud-suite.sh --release DIR --work DIR [--programs DIR]' >&2
  echo '         [--runsc PATH] [--dns ADDRESSES] [--address ADDRESS]' >&2
  echo '         [-- TEST-ARGUMENTS...]' >&2
  exit 2
}
while [ "$#" -gt 0 ]; do
  [ "$1" = -- ] && { shift; break; }
  [ "$#" -ge 2 ] || usage
  case "$1" in
    --release) release=$2 ;;
    --work) work=$2 ;;
    --programs) programs=$2 ;;
    --runsc) runsc=$2 ;;
    --dns) dns=$2 ;;
    --address) address=$2 ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
  shift 2
done
[ -n "$release" ] && [ -n "$work" ] || usage
[ "$(id -u)" = 0 ] || { echo 'run as root' >&2; exit 2; }
[ -f "$release/image/manifest.json" ] || { echo "no Cloud image release at $release/image" >&2; exit 2; }
[ -d "$release/commands" ] || { echo "no command packages at $release/commands" >&2; exit 2; }
# The suite's programs, and the command that runs the suite, which takes the
# suite's arguments last.
if [ -n "$programs" ]; then
  programs="$(cd "$programs" && pwd)"
  needed=(demi-machine-manager demi-server backend-scenarios)
  suite_command=("$programs/backend-scenarios")
else
  programs="$repository/target/debug"
  needed=(demi-machine-manager demi-server)
  suite_command=(cargo test --workspace --features demi-runner/test-fixtures --test backend --)
fi
for name in "${needed[@]}"; do
  [ -x "$programs/$name" ] || { echo "no executable at $programs/$name: build the programs first" >&2; exit 2; }
done
manager="$programs/demi-machine-manager"
server="$programs/demi-server"
# The gVisor version the suite's demi-server pins, which it fetches.
runsc=${runsc:-$("$server" runtime)/runsc}
[ -x "$runsc" ] || { echo "no runsc at $runsc" >&2; exit 2; }
# The Clouds reach the backend on the machine's address toward them.
address=${address:-$(ip -4 route get 1.1.1.1 | sed -n 's/.* src \([0-9.]*\).*/\1/p')}
[ -n "$address" ] || { echo 'no IPv4 address toward the Clouds: pass --address' >&2; exit 2; }
dns=${dns:-$(awk '$1 == "nameserver" && $2 ~ /^[0-9.]+$/ && $2 !~ /^127\./ { print $2; exit }' /etc/resolv.conf)}
[ -n "$dns" ] || { echo 'no resolver for the Clouds: pass --dns' >&2; exit 2; }
# A running manager's command line; pgrep -x compares only the first 15
# characters of a process's name.
manager_pattern='^[^ ]*demi-machine-manager( |$)'
# The firewall table is the manager's: a running manager's would be replaced.
if pgrep -f "$manager_pattern" > /dev/null; then
  echo 'a machine manager runs here already: stop it for the run' >&2
  exit 2
fi
# A stopped manager leaves its table, which it makes again when it starts.
# It goes before the state before the run is recorded, so the run's cleanup
# leaves none and the comparison expects none.
if nft list table inet demi_cloud > /dev/null 2>&1; then
  echo 'cloud-suite: deleting the table inet demi_cloud a stopped manager left' >&2
  nft delete table inet demi_cloud
fi
port=""
for candidate in $(shuf -i 20000-60999 -n 50); do
  if [ -z "$(ss -Hltn "sport = :$candidate")" ]; then
    port=$candidate
    break
  fi
done
[ -n "$port" ] || { echo 'no free port for the backend' >&2; exit 2; }
url="http://$address:$port"

mkdir -p "$work"
work="$(cd "$work" && pwd)"
state="$work/state"
socket="$work/machines.sock"
[ ! -e "$state" ] || { echo "$state exists: a run's state is removed after it" >&2; exit 2; }
rm -f "$socket" "$work/stand-in.ready"

# What a run can leave behind, listed before and after it.
snapshot() {
  local into=$1
  mkdir -p "$into"
  pgrep -a -f '^runsc' | sort > "$into/runsc-processes" || true
  pgrep -a -f "$manager_pattern" | sort > "$into/managers" || true
  awk '{ print $4, $5, $9, $10 }' /proc/self/mountinfo | sort > "$into/mounts"
  for backing in /sys/block/loop*/loop/backing_file; do
    # Without an attached loop device the pattern matches nothing.
    if [ -e "$backing" ]; then
      echo "$backing $(cat "$backing")"
    fi
  done | sort > "$into/loop-devices"
  nft list tables | sort > "$into/nftables-tables"
  ip -o link show | awk -F': ' '{ print $2 }' | sort > "$into/links"
  ip netns list | sort > "$into/network-namespaces"
  find /sys/fs/cgroup -name 'demi*' | sort > "$into/cgroups"
  ss -Hltn "sport = :$port" | sort > "$into/listeners"
  cat /proc/sys/net/ipv4/ip_forward > "$into/ip-forward"
}
snapshot "$work/before"
forwarding=$(cat /proc/sys/net/ipv4/ip_forward)

stand_in=""
stand_in_namespace=""
manager_pid=""
nsenter_pid=""
suite=1
finish() {
  local status=$?
  trap - EXIT INT TERM
  set +e
  if [ -n "$manager_pid" ] && kill -0 "$manager_pid" 2> /dev/null; then
    # The manager's drain saves every Cloud; it has no deadline of its own.
    kill -TERM "$manager_pid"
    for _ in $(seq 600); do
      kill -0 "$manager_pid" 2> /dev/null || break
      sleep 1
    done
  fi
  [ -z "$nsenter_pid" ] || wait "$nsenter_pid" 2> /dev/null
  if [ -n "$stand_in" ]; then
    # unshare's end kills the stand-in's init, and with it every process in
    # its PID namespace.
    kill -KILL "$stand_in" 2> /dev/null
    wait "$stand_in" 2> /dev/null
  fi
  if [ -n "$stand_in_namespace" ]; then
    for _ in $(seq 100); do
      ls -l /proc/[0-9]*/ns/pid 2> /dev/null | grep -qF "$stand_in_namespace" || break
      sleep 0.1
    done
  fi
  nft delete table inet demi_cloud 2> /dev/null
  echo "$forwarding" > /proc/sys/net/ipv4/ip_forward
  rm -rf "$state"
  rm -f "$socket" "$work/stand-in.ready"
  snapshot "$work/after"
  local remains=0
  for list in "$work/before"/*; do
    local name
    name=$(basename "$list")
    if ! diff -u "$list" "$work/after/$name" > "$work/after/$name.diff"; then
      echo "cloud-suite: $name differs after the run:" >&2
      cat "$work/after/$name.diff" >&2
      remains=1
    fi
  done
  if [ -n "$stand_in_namespace" ] && ls -l /proc/[0-9]*/ns/pid 2> /dev/null | grep -qF "$stand_in_namespace"; then
    echo "cloud-suite: processes of the stand-in host remain" >&2
    remains=1
  fi
  if [ "$remains" = 0 ]; then
    echo 'cloud-suite: nothing the run made remains' >&2
  fi
  if [ "$status" = 0 ] && { [ "$suite" != 0 ] || [ "$remains" != 0 ]; }; then
    status=1
  fi
  exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Linux 6.18 numbers namespaces from per-CPU batches (gen_cookie_next in
# kernel/nstree.c), and mnt_ns_loop in fs/namespace.c binds a mount namespace
# only into one with a lower number, so the manager's pin, which binds its
# namespace into the stand-in's, would fail at random. Namespaces made on one
# CPU are numbered in the order they are made: the stand-in's namespace and
# the manager's are made on the same CPU, and the manager then runs on every
# CPU this script may use. A real host's namespace is the initial one,
# numbered before all others.
cpus=$(taskset -pc $$ | sed 's/.*: //')
cpu=${cpus%%[-,]*}

# The stand-in host: its mounts are shared, as systemd's are; /run and the
# cgroup root are its own. Its namespace is made on $cpu (above). Its init
# reaps the processes it adopts, as a host's init does: runsc takes a dead
# Sentry that nobody reaped for a running one, and could not stop it.
taskset -c "$cpu" unshare --mount --pid --fork --mount-proc --kill-child -- bash -c '
  set -e
  mount --make-rshared /
  mount -t tmpfs -o mode=0755 tmpfs /run
  mount -t tmpfs -o ro,mode=0755 tmpfs /sys/fs/cgroup
  echo ready > "$1"
  # bash reaps every child that ends while it waits, adopted ones included;
  # the loop outlives any one sleep.
  while :; do
    sleep 3600 &
    wait $! || :
  done' stand-in "$work/stand-in.ready" &
stand_in=$!
for _ in $(seq 100); do
  [ -e "$work/stand-in.ready" ] && break
  kill -0 "$stand_in" 2> /dev/null || { echo 'the stand-in host did not start' >&2; exit 1; }
  sleep 0.1
done
[ -e "$work/stand-in.ready" ] || { echo 'the stand-in host did not start' >&2; exit 1; }
stand_in_namespace=$(readlink "/proc/$stand_in/ns/pid_for_children")

# The manager, as its unit starts it: in a mount namespace of its own that
# receives the host's mounts, made on $cpu after the stand-in's (above).
nsenter --mount="/proc/$stand_in/ns/mnt" --pid="/proc/$stand_in/ns/pid_for_children" -- \
  taskset -c "$cpu" unshare --mount --propagation slave -- \
  taskset -c "$cpus" env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin \
  DEMI_RELEASE="$release" \
  DEMI_MACHINE_MANAGER_SOCKET="$socket" \
  DEMI_MANAGED_DATA="$state" \
  DEMI_MANAGED_RUNSC="$runsc" \
  DEMI_BACKEND_PUBLIC_URL="$url" \
  DEMI_MANAGED_DNS="$dns" \
  DEMI_MANAGED_SLOTS=4 \
  DEMI_MANAGED_LIMITS=off \
  "$manager" > "$work/manager.log" 2>&1 &
nsenter_pid=$!
# nsenter forks the process that enters the PID namespace, which becomes the
# manager.
for _ in $(seq 100); do
  manager_pid=$(cat "/proc/$nsenter_pid/task/$nsenter_pid/children" 2> /dev/null | awk '{ print $1 }')
  [ -n "$manager_pid" ] && break
  sleep 0.1
done
[ -n "$manager_pid" ] || { echo 'nsenter started no manager' >&2; exit 1; }
# A first start imports the image, which takes minutes; the wait ends when
# the manager is ready or exits.
until grep -q 'gVisor/systrap ready at' "$work/manager.log"; do
  if ! kill -0 "$manager_pid" 2> /dev/null; then
    echo 'the manager exited before it was ready:' >&2
    cat "$work/manager.log" >&2
    exit 1
  fi
  sleep 1
done
echo "cloud-suite: the manager is ready; the backend URL is $url" >&2

set +e
(
  # cargo test runs a test executable in its package's directory.
  cd "$repository/crates/backend"
  DEMI_TEST_MACHINES_SOCKET="$socket" \
    DEMI_TEST_CLOUD_URL="$url" \
    DEMI_TEST_MACHINES_DATA="$state" \
    DEMI_TEST_CLOUD_RELEASE="$release" \
    "${suite_command[@]}" --include-ignored real_cloud --test-threads=1 --nocapture "$@"
) 2>&1 | tee "$work/suite.log"
suite=${PIPESTATUS[0]}
set -e
exit "$suite"
