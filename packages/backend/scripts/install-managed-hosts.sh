#!/usr/bin/env bash
# The one-time, root, install-time setup for managed hosts on a backend
# machine (managed-hosts.md § Provisioning): the tap pool with a /30 per
# slot, IP forwarding and NAT for the guests' egress, and the egress policy
# as nftables rules the guests cannot alter. Idempotent: rerun after a
# reboot or to change the pool.
#
#   sudo install-managed-hosts.sh --user demi-backend --backend-address 172.16.0.1 --backend-port 3271 \
#        [--mode direct|jailer] [--uid-base 20000] [--subnet 172.16.0.0/16] [--slots 256] [--egress-iface eth0]
set -euo pipefail
user=""; backend_address=""; backend_port=""; mode=direct; uid_base=20000; subnet=172.16.0.0/16; slots=256; egress=""; prefix=demi
while [ $# -gt 0 ]; do
  case "$1" in
    --user) user=$2; shift 2 ;;
    --backend-address) backend_address=$2; shift 2 ;;
    --backend-port) backend_port=$2; shift 2 ;;
    --mode) mode=$2; shift 2 ;;
    --uid-base) uid_base=$2; shift 2 ;;
    --subnet) subnet=$2; shift 2 ;;
    --slots) slots=$2; shift 2 ;;
    --egress-iface) egress=$2; shift 2 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$user" ] && [ -n "$backend_address" ] && [ -n "$backend_port" ] || { echo "--user, --backend-address and --backend-port are required" >&2; exit 2; }
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 2; }
[ -n "$egress" ] || egress=$(ip -o route show default | awk '{print $5; exit}')

ip_to_int() { local IFS=.; read -r a b c d <<<"$1"; echo $(( (a<<24) + (b<<16) + (c<<8) + d )); }
int_to_ip() { echo "$(( ($1>>24)&255 )).$(( ($1>>16)&255 )).$(( ($1>>8)&255 )).$(( $1&255 ))"; }
network=${subnet%/*}
base=$(ip_to_int "$network")

for ((i=0; i<slots; i++)); do
  tap="$prefix$i"
  if [ "$mode" = jailer ]; then owner=$((uid_base + i)); else owner=$user; fi
  # Recreated every run: the owner follows the mode, and a tap's owner cannot change in place.
  ip link show "$tap" >/dev/null 2>&1 && ip link del "$tap"
  ip tuntap add "$tap" mode tap user "$owner"
  gw=$(int_to_ip $((base + i*4 + 1)))
  ip addr replace "$gw/30" dev "$tap"
  ip link set "$tap" up
done

sysctl -qw net.ipv4.ip_forward=1

# /dev/kvm for the backend user (direct mode spawns Firecracker as it; jailer mode's helper runs as root anyway):
# the kvm group, effective at the user's next login.
getent group kvm >/dev/null && usermod -aG kvm "$user"

# The egress policy, one table, replaced whole so reruns converge.
nft -f - <<RULES
table inet demi
delete table inet demi
table inet demi {
  set private { type ipv4_addr; flags interval; elements = { 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, 127.0.0.0/8, 100.64.0.0/10 } }
  chain forward {
    type filter hook forward priority 0; policy accept;
    iifname "$prefix*" ct state established,related accept
    iifname "$prefix*" ip daddr $backend_address accept
    iifname "$prefix*" ip daddr @private drop
    iifname "$prefix*" accept
    oifname "$prefix*" ct state established,related accept
    oifname "$prefix*" drop
  }
  chain input {
    type filter hook input priority 0; policy accept;
    iifname "$prefix*" ct state established,related accept
    iifname "$prefix*" ip daddr $backend_address tcp dport $backend_port accept
    iifname "$prefix*" ip daddr $backend_address udp dport 53 accept
    iifname "$prefix*" drop
  }
  chain postrouting {
    type nat hook postrouting priority 100; policy accept;
    ip saddr $subnet oifname "$egress" masquerade
  }
}
RULES
echo "managed hosts: $slots taps (${prefix}0..${prefix}$((slots-1))) on $subnet, mode $mode, egress via $egress, backend $backend_address:$backend_port"
