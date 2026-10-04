// Command demi-backend serves the hosted Demi product.
package main

import (
	"context"
	"os"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/version"
)

func main() {
	os.Exit(backend.Main(context.Background(), version.Release))
}
