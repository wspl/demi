// Command demi-claude-code serves the Claude Code installation operations.
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/wspl/demi/internal/cmdpkg/claudecode"
)

func main() {
	if err := claudecode.Serve(context.Background(), os.Args[1:]); err != nil {
		// A failed diagnostic write cannot change the required failure status.
		_, _ = fmt.Fprintf(os.Stderr, "demi-claude-code: %v\n", err)
		os.Exit(1)
	}
	os.Exit(0)
}
