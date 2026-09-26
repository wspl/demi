package browser

import (
	"testing"

	"github.com/wspl/demi/internal/contract/contracttest"
)

// The recorded corpus: Go accepts what Zod accepts, rejects what it rejects,
// and encodes an accepted value to the bytes TypeScript writes.
func TestCorpus(t *testing.T) {
	contracttest.Run(t, "testdata/corpus.json", corpusRoots)
}
