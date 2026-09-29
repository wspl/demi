package main

import (
	"fmt"
	"github.com/wspl/demi/go/webapi/wiredecl"
	"os"
)

func main() {
	if err := wiredecl.Generate("."); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
