package main

import (
	"context"
	"fmt"

	"github.com/wspl/demi/internal/framewire"
)

const (
	recallWindow     = 200000
	extraGenerations = 3
)

type recallRow struct {
	generations int
	recalled    int
}

func recall(ctx context.Context, c *conversation) (bool, error) {
	baseline := c.generations()
	fmt.Println("\n── baseline recall")
	first, err := c.recall(ctx)
	if err != nil {
		return false, err
	}
	rows := []recallRow{{generations: baseline, recalled: first}}
	for extra := 1; extra <= extraGenerations; extra++ {
		if rows[len(rows)-1].recalled < 3 {
			break
		}
		fmt.Printf("\n── extra compact #%d/%d\n", extra, extraGenerations)
		added, err := forceCompaction(ctx, c, extra)
		if err != nil {
			return false, err
		}
		if !added {
			break
		}
		generations := c.generations()
		count, err := c.recall(ctx)
		if err != nil {
			return false, err
		}
		rows = append(rows, recallRow{generations: generations, recalled: count})
	}
	errors := c.errors()
	fmt.Println("\n===== LONG-SESSION COMPACTION VERIFY =====")
	fmt.Println("generations | recall")
	passed := baseline >= 2 && errors == 0
	for _, row := range rows {
		fmt.Printf("%11d | %d/3\n", row.generations, row.recalled)
		passed = passed && row.recalled == 3
	}
	fmt.Printf("error blocks: %d\n", errors)
	printResult(passed)
	return passed, nil
}

func forceCompaction(ctx context.Context, c *conversation, extra int) (bool, error) {
	before := c.generations()
	if err := c.grow(ctx, fmt.Sprintf("VERIFY-%d", extra), 6000); err != nil {
		return false, err
	}
	if err := c.act(ctx, &framewire.CompactFrame{}); err != nil {
		return false, err
	}
	if c.generations() <= before {
		fmt.Println("   compact added no generation; growing harder and retrying once")
		if err := c.grow(ctx, fmt.Sprintf("FORCE-%d", extra), 20000); err != nil {
			return false, err
		}
		if err := c.act(ctx, &framewire.CompactFrame{}); err != nil {
			return false, err
		}
	}
	if c.generations() <= before {
		fmt.Println("   still no new generation after the retry: stopping")
		return false, nil
	}
	fmt.Printf("   compacted: %d → %d generations\n", before, c.generations())
	return true, nil
}
