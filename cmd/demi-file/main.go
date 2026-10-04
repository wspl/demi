// Command demi-file serves native file operations.
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/wspl/demi/internal/commandpackage/file"
)

func main() {
	if err := file.Serve(context.Background(), os.Args[1:]); err != nil {
		// A failed diagnostic write cannot change the required exit status.
		_, _ = fmt.Fprintf(os.Stderr, "demi-file: %v\n", err)
		os.Exit(1)
	}
	os.Exit(0)
}
