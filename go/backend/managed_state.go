package backend

import "context"

// Cloud is the user's Cloud machine and its reset intent (the managed
// Clouds part, G7h).
type Cloud struct{}

// stop stops the Cloud's timers (a close step).
func (*Cloud) stop() {}

// saveCloud saves and stops the user's Cloud, and answers why it was not
// saved, or "" if it was (a close step, run off the shard).
func saveCloud(context.Context, ShardRef) string { return "" }
