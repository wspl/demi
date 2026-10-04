// Command compactionfixture checks recorded secrets against a real model.
// No automated test runs this program.
package main

import (
	"context"
	"errors"
	"fmt"
	"os"
)

func main() {
	passed, err := run(context.Background())
	if err != nil {
		fmt.Fprintf(os.Stderr, "compaction fixture: %v\n", err)
	}
	if err != nil || !passed {
		os.Exit(1)
	}
}

func run(ctx context.Context) (passed bool, err error) {
	mode := "recall"
	if len(os.Args) > 1 {
		mode = os.Args[1]
	}
	f, err := loadFixture()
	if err != nil {
		return false, err
	}
	switch mode {
	case "recall":
		fmt.Printf("loaded fixture: total≈%d tokens, %d blocks, %d generations (extra=%d)\n",
			f.BuiltTokens, len(f.Blocks), f.Generations, extraGenerations)
		c, err := openConversation(ctx, f, flash(recallWindow))
		if err != nil {
			return false, err
		}
		defer func() {
			err = errors.Join(err, c.close(context.WithoutCancel(ctx)))
		}()
		return recall(ctx, c)
	case "switch":
		small, err := window("COMPACTION_FIXTURE_SMALL_WINDOW", 8000)
		if err != nil {
			return false, err
		}
		large, err := window("COMPACTION_FIXTURE_LARGE_WINDOW", 400000)
		if err != nil {
			return false, err
		}
		c, err := openConversation(ctx, f, flash(small))
		if err != nil {
			return false, err
		}
		defer func() {
			err = errors.Join(err, c.close(context.WithoutCancel(ctx)))
		}()
		return switchWindows(ctx, c, small, large)
	default:
		return false, fmt.Errorf("unknown mode %q: recall or switch", mode)
	}
}

func printResult(passed bool) {
	if passed {
		fmt.Println("PASSED")
		return
	}
	fmt.Println("FAILED")
}
