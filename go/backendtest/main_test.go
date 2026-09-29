package backendtest_test

import (
	"os"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

func TestMain(m *testing.M) {
	os.Exit(backendtest.Main(m))
}
