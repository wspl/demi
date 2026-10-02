// Package hostaccess resolves conversation Hosts and holds their file gates for
// operations, transfers, and shell jobs. It owns the slots, transitions and edge
// leases through HostShard; it never imports the concrete user's shard.
//
// This is the b-hostaccess API checkpoint. Function bodies deliberately panic
// until the implementation checkpoint. It compiles for dependent API work but
// must not be used to serve requests yet.
package hostaccess
