package commandprototest

// FixtureOperations returns the native fixture service's operation catalog.
func FixtureOperations() []string {
	return []string{
		"where",
		"echo",
		"first",
		"spin",
		"result",
		"retain",
		"stall_release",
		"held",
		"crash",
		"stalled",
		"proceed",
		"number",
	}
}
