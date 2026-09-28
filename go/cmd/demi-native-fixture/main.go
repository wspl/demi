package main

import (
	"context"
	"fmt"
	"os"

	cs "github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

func main() {
	if err := cs.ServeStdio(context.Background(), servicetest.NewFixture()); err != nil {
		// A broken diagnostic pipe cannot prevent the required process exit.
		fmt.Fprintf(os.Stderr, "fixture: %v\n", err)
		os.Exit(1)
	}
}
