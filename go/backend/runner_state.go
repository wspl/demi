package backend

// Devices are the user's devices, each with its runner connection (the
// runners part, G7g).
type Devices struct{}

// disconnectAll closes every runner connection with reason; their work ends
// with them (a close step).
func (*Devices) disconnectAll(string) {}

// CommandRouter routes each agent node's commands to the rpc calls of its
// jobs (G7g).
type CommandRouter struct{}
