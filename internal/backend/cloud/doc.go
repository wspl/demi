// Package cloud manages each user's Cloud: its capacity, machine transitions,
// machine access and manager connection. Operations accept CloudShard, the
// consuming interface implemented by the user's shard, without knowing its
// conversations or exposes. Conversation Host work enters through hostaccess;
// Access is only for work on the user's machine itself.
//
// This is the b-cloud API checkpoint. Function and method bodies deliberately
// panic with "not written: b-cloud" until the implementation checkpoint.
package cloud
