// Package runners owns pairing, device connections and Host handles, conversation
// file leases, command routing, installers and native artifact publication.
// Conversation Host handles are made only against a FileLease held by host access.
//
// This is the b-runners API checkpoint. Function bodies are supplied in the
// implementation checkpoint; calling them currently panics.
package runners
