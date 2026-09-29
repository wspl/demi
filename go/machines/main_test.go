//go:build linux

package machines_test

import (
	"os"
	"testing"

	"github.com/wspl/demi/go/machines/internal/roottest"
)

func TestMain(m *testing.M) {
	os.Exit(roottest.Main(m))
}
