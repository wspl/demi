package backend

import "context"

// Conversations are the user's conversations, each with its file gate and
// transfers (the conversations part, G7g).
type Conversations struct{}

// Titles are the title requests of the user's conversations (G7g).
type Titles struct{}

// abortAll aborts every title request (a close step).
func (*Titles) abortAll() {}

// ConversationAgent is the user's conversation trees, which go/agent serves
// (G7g, on G7d's API).
type ConversationAgent struct{}

// endTransfers ends the open file transfers and user streams, which stay
// closed, and waits for them (a close step, run off the shard).
func endTransfers(context.Context, ShardRef) {}

// shutDownAgent aborts the agent's turns while their runners are still
// connected, and waits for them (a close step, run off the shard).
func shutDownAgent(context.Context, ShardRef) {}
