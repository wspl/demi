// Command demi-runner runs this device as a Demi execution target.
package main

import (
	"os"

	"github.com/wspl/demi/internal/runner"
	"github.com/wspl/demi/internal/version"
)

func main() {
	os.Exit(runner.Main(version.Release))
}
