// Command demi-native-fixture serves deliberately faulty operations for integration tests.
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runner/cmdpkgs/cmdpkgstest"
)

func main() {
	if err := cmdsdk.ServeStdio(context.Background(), &cmdpkgstest.Fixture{}); err != nil {
		_, _ = fmt.Fprintln(os.Stderr, "fixture:", err) // Failure to report cannot change the required exit status.
		os.Exit(1)
	}
}
