// Package hostaccess resolves conversation Hosts and holds their file gates for
// operations, transfers, and shell jobs. It owns the slots, transitions and edge
// leases through HostShard; it never imports the concrete user's shard.
package hostaccess
