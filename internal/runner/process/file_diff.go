package process

import (
	"unicode/utf8"

	"github.com/sergi/go-diff/diffmatchpatch"
	"github.com/wspl/demi/internal/commandwire"
)

// LineCounts returns lines added and removed using an exact line Myers diff.
// A nil side is absent; binary or non-UTF-8 content yields zero counts.
// Callers represent unread or oversized content according to their own limits.
func LineCounts(before, after []byte) (added, removed uint64) {
	if !commandwire.IsText(before) || !commandwire.IsText(after) {
		return 0, 0
	}
	d := diffmatchpatch.New()
	d.DiffTimeout = 0
	a, b, _ := d.DiffLinesToRunes(string(before), string(after))
	for _, change := range d.DiffMainRunes(a, b, false) {
		switch change.Type {
		case diffmatchpatch.DiffInsert:
			added += uint64(utf8.RuneCountInString(change.Text))
		case diffmatchpatch.DiffDelete:
			removed += uint64(utf8.RuneCountInString(change.Text))
		}
	}
	return added, removed
}
