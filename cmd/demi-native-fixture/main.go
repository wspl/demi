// Command demi-native-fixture serves deliberately faulty operations for integration tests.
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/runner/commandpackages/commandpackagestest"
)

func main() {
	if err := commandsdk.ServeStdio(context.Background(), &commandpackagestest.Fixture{}); err != nil {
		_, _ = fmt.Fprintln(os.Stderr, "fixture:", err) // Failure to report cannot change the required exit status.
		os.Exit(1)
	}
}
