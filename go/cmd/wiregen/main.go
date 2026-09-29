// Command wiregen writes the decoders, the rule checks and the encoders of the
// wire types that the Go package in the current directory declares, and,
// with -ts, their TypeScript schemas. It runs under go generate:
//
//	//go:generate go run github.com/wspl/demi/go/cmd/wiregen
//
// The design is docs/internal/go-migration/design/wire-contracts.md.
package main

import (
	"flag"
	"fmt"
	"os"

	"github.com/wspl/demi/go/internal/wiregen"
)

func main() {
	ts := flag.String("ts", "", "write the TypeScript schemas to `file`")
	flag.Parse()
	if err := wiregen.Write(".", *ts); err != nil {
		fmt.Fprintf(os.Stderr, "wiregen: %v\n", err)
		os.Exit(1)
	}
}
