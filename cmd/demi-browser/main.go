// Command demi-browser serves the resident conversation browser package.
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/wspl/demi/internal/commandpackage/browser"
)

func main() {
	if err := browser.Serve(context.Background(), os.Args[1:]); err != nil {
		// A failed diagnostic write cannot change the required failure exit status.
		_, _ = fmt.Fprintln(os.Stderr, "demi-browser:", err)
		os.Exit(1)
	}
	// The SDK owns synchronous Windows stdio workers.
	os.Exit(0)
}
