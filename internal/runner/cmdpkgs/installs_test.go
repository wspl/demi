package cmdpkgs

import (
	"testing"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runnerwire"
)

func TestInstallChangesAtEachHundredthAndLeavesWhenDone(t *testing.T) {
	var installs Installs
	defer installs.Close()
	reported := installs.Subscribe()
	installing := installs.start(
		Wanted{
			Package:  "demi.browser",
			Name:     "program",
			Version:  "0.1.3",
			Artifact: commandwire.PackageArtifact{Size: 1000},
		},
	)
	select {
	case <-reported.seen:
	default:
		t.Fatal("starting install did not notify subscriber")
	}
	if got := reported.Current(); len(got) != 1 || got[0].Done != 0 {
		t.Fatalf("initial: %v", got)
	}
	installing.downloaded(9)
	select {
	case <-reported.seen:
		t.Fatal("changed before first hundredth")
	default:
	}
	installing.downloaded(10)
	if got := reported.Current()[0].Done; got != 10 {
		t.Fatalf("done=%d", got)
	}
	installing.downloaded(15)
	select {
	case <-reported.seen:
		t.Fatal("changed within hundredth")
	default:
	}
	installing.downloaded(1000)
	if got := reported.Current()[0].Done; got != 1000 {
		t.Fatalf("done=%d", got)
	}
	installing.unpacking()
	if got := reported.Current()[0].Phase; got != runnerwire.InstallPhaseUnpack {
		t.Fatalf("phase=%s", got)
	}
	installing.close()
	if len(reported.Current()) != 0 {
		t.Fatal("completed install remains")
	}
}
