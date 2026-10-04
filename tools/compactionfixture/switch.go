package main

import (
	"context"
	"errors"
	"fmt"
)

const switchMinContext = 8000

type switchResults struct {
	noCompactionUp bool
	compactedDown  bool
	recallUp       int
	recallDown     int
	recallBack     int
}

func switchWindows(ctx context.Context, c *conversation, small, large uint32) (bool, error) {
	estimate, err := c.context(large)
	if err != nil {
		return false, err
	}
	fmt.Printf("loaded: %d blocks, ctx≈%d replayable tokens, %d generations; windows small=%d large=%d\n",
		len(c.blocks()), estimate, c.generations(), small, large)
	fmt.Println("\n── STEP 1: switch small → large, expecting no compaction")
	before := c.generations()
	if err := c.actSwitch(ctx, flash(large)); err != nil {
		return false, err
	}
	var results switchResults
	results.recallUp, err = c.recall(ctx)
	if err != nil {
		return false, err
	}
	results.noCompactionUp = c.generations() == before
	fmt.Printf("   compacted on the larger window: %t\n", !results.noCompactionUp)
	fmt.Println("\n── STEP 2: grow until the small window must compact")
	if err := fillSmallWindow(ctx, c, small, large); err != nil {
		return false, err
	}
	if err := switchDownAndBack(ctx, c, small, large, &results); err != nil {
		return false, err
	}
	return results.report(c.errors()), nil
}

func fillSmallWindow(ctx context.Context, c *conversation, small, large uint32) error {
	threshold := uint64(small) * 8 / 10
	turns := 0
	for {
		smallEstimate, err := c.context(small)
		if err != nil {
			return err
		}
		largeEstimate, err := c.context(large)
		if err != nil {
			return err
		}
		if smallEstimate >= threshold && largeEstimate >= switchMinContext {
			fmt.Printf("   ready after %d turns: estimate@small=%d (threshold %d)\n", turns, smallEstimate, threshold)
			return nil
		}
		if err := c.grow(ctx, fmt.Sprintf("FILLER-%d", turns), 8000); err != nil {
			return err
		}
		turns++
		if turns > 40 {
			return errors.New("40 filler turns did not fill the small window")
		}
	}
}

func switchDownAndBack(ctx context.Context, c *conversation, small, large uint32, results *switchResults) error {
	fmt.Println("\n── STEP 3: switch large → small, expecting a compaction by the model before the switch")
	before := c.generations()
	if err := c.actSwitch(ctx, flash(small)); err != nil {
		return err
	}
	var err error
	results.recallDown, err = c.recall(ctx)
	if err != nil {
		return err
	}
	results.compactedDown = c.generations() > before
	fmt.Printf("   compacted for the smaller window: %t\n", results.compactedDown)
	fmt.Println("\n── STEP 4: switch back to large")
	if err := c.actSwitch(ctx, flash(large)); err != nil {
		return err
	}
	results.recallBack, err = c.recall(ctx)
	return err
}

func (r switchResults) report(errors int) bool {
	fmt.Println("\n===== WINDOW-SWITCH COMPACTION VERIFY =====")
	fmt.Printf("step 1  small→large: no compaction = %t, recall %d/3\n", r.noCompactionUp, r.recallUp)
	fmt.Printf("step 3  large→small: compaction = %t, recall %d/3\n", r.compactedDown, r.recallDown)
	fmt.Printf("step 4  switch back: recall %d/3\n", r.recallBack)
	fmt.Printf("error blocks: %d\n", errors)
	passed := r.noCompactionUp && r.compactedDown && r.recallUp == 3 && r.recallDown == 3 &&
		r.recallBack == 3 && errors == 0
	printResult(passed)
	return passed
}
