// Command demi-native-fixture is the runner's native fixture service: a
// command service over its standard input and output, with deliberately faulty
// operations for the runner's integration tests. The runner starts it with
// --command-service, which it needs no other flag for.
package main

import "github.com/wspl/demi/go/commandservice/fixture"

func main() {
	fixture.Main()
}
