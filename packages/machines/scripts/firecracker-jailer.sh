#!/usr/bin/env bash
# Privileged Firecracker setup. The backend waits for this process for the VM's
# lifetime.
set -euo pipefail

fail() {
  printf 'firecracker-jailer: %s\n' "$*" >&2
  exit 2
}

usage() {
  fail 'usage: firecracker-jailer.sh vm start --id ID --jailer PATH --firecracker PATH --chroot-base DIR --uid N --gid N --backend-gid N --kernel PATH --rootfs PATH --home PATH --system PATH | vm kill --id ID --chroot-base DIR'
}

require_absolute_path() {
  local flag=$1
  local value=$2
  case "$value" in
    /*)
      ;;
    *)
      fail "$flag must be an absolute path"
      ;;
  esac
  case "$value/" in
    */../*)
      fail "$flag must not contain .."
      ;;
  esac
}

require_number() {
  local flag=$1
  local value=$2
  local minimum=$3
  # Bound the decimal string before using shell arithmetic.
  [[ "$value" =~ ^[0-9]{1,10}$ ]] || fail "$flag must be an integer"
  local number=$((10#$value))
  if (( number < minimum || number > 4294967294 )); then
    fail "$flag must be between $minimum and 4294967294"
  fi
}

parse_arguments() {
  (( $# >= 2 )) || usage
  [[ "$1" == vm ]] || usage
  action=$2
  [[ "$action" == start || "$action" == kill ]] || usage
  shift 2

  local seen=' '
  local flag value
  while (( $# > 0 )); do
    flag=$1
    (( $# >= 2 )) || fail "missing value for $flag"
    value=$2
    [[ "$seen" != *" $flag "* ]] || fail "duplicate argument $flag"
    seen+="$flag "

    case "$action:$flag" in
      start:--id|kill:--id)
        vm_id=$value
        ;;
      start:--chroot-base|kill:--chroot-base)
        chroot_base=$value
        ;;
      start:--jailer)
        jailer=$value
        ;;
      start:--firecracker)
        firecracker=$value
        ;;
      start:--uid)
        vm_uid=$value
        ;;
      start:--gid)
        vm_gid=$value
        ;;
      start:--backend-gid)
        backend_gid=$value
        ;;
      start:--kernel)
        kernel=$value
        ;;
      start:--rootfs)
        rootfs=$value
        ;;
      start:--home)
        home_image=$value
        ;;
      start:--system)
        system_image=$value
        ;;
      *)
        fail "unknown argument $flag"
        ;;
    esac
    shift 2
  done

  [[ "$vm_id" =~ ^[A-Za-z0-9_-]{1,64}$ ]] || fail '--id must be [A-Za-z0-9_-]{1,64}'
  require_absolute_path --chroot-base "$chroot_base"
  if [[ "$action" == kill ]]; then
    return
  fi

  require_absolute_path --jailer "$jailer"
  require_absolute_path --firecracker "$firecracker"
  require_absolute_path --kernel "$kernel"
  require_absolute_path --rootfs "$rootfs"
  require_absolute_path --home "$home_image"
  require_absolute_path --system "$system_image"
  require_number --uid "$vm_uid" 1000
  require_number --gid "$vm_gid" 1000
  require_number --backend-gid "$backend_gid" 1

  [[ -x "$jailer" && -f "$jailer" ]] || fail '--jailer must name an executable file'
  [[ -x "$firecracker" && -f "$firecracker" ]] || fail '--firecracker must name an executable file'
  for value in "$kernel" "$rootfs" "$home_image" "$system_image"; do
    [[ -f "$value" ]] || fail "image does not exist: $value"
  done
}

read_pid() {
  local path=$1
  local text
  [[ -r "$path" ]] || return 1
  text=$(< "$path")
  [[ "$text" =~ ^[[:space:]]*([0-9]{1,10})[[:space:]]*$ ]] || return 1
  local pid=$((10#${BASH_REMATCH[1]}))
  (( pid > 1 && pid <= 2147483647 )) || return 1
  printf '%s\n' "$pid"
}

process_alive() {
  local pid=$1
  local stat_record state
  kill -0 "$pid" 2>/dev/null || return 1
  # A dead namespace init can remain a zombie until its host parent reaps it.
  if [[ -r "/proc/$pid/stat" ]]; then
    IFS= read -r stat_record < "/proc/$pid/stat" || return 1
    stat_record=${stat_record##*) }
    state=${stat_record%% *}
    [[ "$state" != Z && "$state" != X ]] || return 1
  fi
}

link_or_copy() {
  local source=$1
  local destination=$2
  if ! ln -- "$source" "$destination" 2>/dev/null; then
    cp -- "$source" "$destination"
  fi
}

write_pid_record() {
  printf '%s\n' "$1" > "$jail/pid.next"
  mv -- "$jail/pid.next" "$jail/pid"
}

force_stop() {
  local pid=$1
  if ! process_alive "$pid"; then
    return
  fi
  if ! kill -KILL "$pid" 2>/dev/null; then
    # The process can exit between the liveness check and the signal.
    if process_alive "$pid"; then
      printf 'firecracker-jailer: cannot stop process %s\n' "$pid" >&2
      return 1
    fi
  fi
  while process_alive "$pid"; do
    sleep 0.05
  done
}

cleanup_start() {
  local status=$?
  local cleanup_failed=0
  trap - EXIT HUP INT TERM
  set +e

  # Stop the launcher first, so it cannot create another child after cleanup
  # starts.
  if [[ -n "$launcher_pid" ]]; then
    force_stop "$launcher_pid" || cleanup_failed=1
    # A launcher killed during cleanup is expected to have a nonzero wait
    # status.
    wait "$launcher_pid" 2>/dev/null
  fi

  if [[ -z "$vm_pid" ]]; then
    vm_pid=$(read_pid "$namespace_pid_file") || vm_pid=''
    if [[ -z "$vm_pid" && -e "$namespace_pid_file" ]]; then
      printf 'firecracker-jailer: cannot read VM PID from %s\n' \
        "$namespace_pid_file" >&2
      cleanup_failed=1
    fi
  fi
  if [[ -n "$vm_pid" ]]; then
    force_stop "$vm_pid" || cleanup_failed=1
  fi

  if (( cleanup_failed == 0 )); then
    if ! rm -rf -- "$jail"; then
      printf 'firecracker-jailer: cannot remove jail %s\n' "$jail" >&2
      cleanup_failed=1
    fi
  else
    printf \
      'firecracker-jailer: leaving jail intact because VM shutdown could not be confirmed: %s\n' \
      "$jail" >&2
  fi
  if (( status == 0 && cleanup_failed != 0 )); then
    status=2
  fi
  exit "$status"
}

start_vm() {
  local exec_name=${firecracker##*/}
  [[ -n "$exec_name" && "$exec_name" != . && "$exec_name" != .. ]] ||
    fail '--firecracker has no file name'
  jail="$chroot_base/$exec_name/$vm_id"
  local root="$jail/root"
  local run="$root/run"
  namespace_pid_file="$root/$exec_name.pid"

  # Never remove an existing VM's directory to make room for another invocation.
  mkdir -p -- "$chroot_base/$exec_name"
  mkdir -- "$jail" || fail "jail already exists or cannot be created: $jail"
  trap cleanup_start EXIT
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM

  mkdir -p -- "$run"
  link_or_copy "$kernel" "$root/vmlinux"
  link_or_copy "$rootfs" "$root/rootfs.ext4"

  # Writable disks must share their inode with the backend's working files.
  ln -- "$home_image" "$root/home.ext4"
  ln -- "$system_image" "$root/system.ext4"
  chown -- "$vm_uid:$backend_gid" "$root/home.ext4" "$root/system.ext4" "$run"
  chmod 660 -- "$root/home.ext4" "$root/system.ext4"
  chmod 2770 -- "$run"
  chown -- "$vm_uid:$vm_gid" "$root"

  local args=(
    --id "$vm_id"
    --exec-file "$firecracker"
    --uid "$vm_uid"
    --gid "$vm_gid"
    --chroot-base-dir "$chroot_base"
    --cgroup-version 2
    --new-pid-ns
  )
  umask 007
  "$jailer" "${args[@]}" &
  launcher_pid=$!
  write_pid_record "$launcher_pid"

  local launcher_status=0
  wait "$launcher_pid" || launcher_status=$?
  launcher_pid=''
  if (( launcher_status != 0 )); then
    exit "$launcher_status"
  fi

  # --new-pid-ns makes the launcher exit; the jailer's file names the actual VM.
  local attempt
  for (( attempt = 0; attempt < 40; attempt++ )); do
    if vm_pid=$(read_pid "$namespace_pid_file"); then
      break
    fi
    sleep 0.05
  done
  [[ -n "$vm_pid" ]] || fail 'jailer exited without recording the VM PID'
  write_pid_record "$vm_pid"

  local socket="$run/firecracker.socket"
  for (( attempt = 0; attempt < 40; attempt++ )); do
    [[ -S "$socket" ]] && break
    process_alive "$vm_pid" || fail 'VM exited before creating its API socket'
    sleep 0.05
  done
  [[ -S "$socket" ]] || fail 'VM did not create its API socket'

  # The jailer replaces directory modes. Restore the backend's API access
  # afterward.
  chgrp -- "$backend_gid" "$root" "$run" "$socket"
  chmod 750 -- "$root" "$run"
  chmod 660 -- "$socket"

  while process_alive "$vm_pid"; do
    sleep 0.1
  done
}

kill_vm() {
  local exec_directory jail_directory pid pid_file
  local exit_status=1
  for exec_directory in "$chroot_base"/*; do
    [[ -d "$exec_directory" ]] || continue
    jail_directory="$exec_directory/$vm_id"
    # Prefer the namespace PID even during the launcher's handoff to the VM.
    pid_file="$jail_directory/root/${exec_directory##*/}.pid"
    if ! pid=$(read_pid "$pid_file"); then
      pid=$(read_pid "$jail_directory/pid") || continue
    fi
    if process_alive "$pid"; then
      kill -KILL "$pid"
      exit_status=0
    fi
  done
  return "$exit_status"
}

action=''
vm_id=''
chroot_base=''
jailer=''
firecracker=''
vm_uid=''
vm_gid=''
backend_gid=''
kernel=''
rootfs=''
home_image=''
system_image=''
jail=''
namespace_pid_file=''
launcher_pid=''
vm_pid=''
parse_arguments "$@"
if [[ "$action" == start ]]; then
  start_vm
else
  kill_vm
fi
