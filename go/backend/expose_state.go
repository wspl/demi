package backend

// Exposes are the user's exposes with relayed connections open (the exposes
// part, G7g).
type Exposes struct{}

// endAll ends the relayed expose connections (a close step).
func (*Exposes) endAll() {}
