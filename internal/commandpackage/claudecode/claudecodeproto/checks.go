package claudecodeproto

import (
	"errors"
	"fmt"
	"strconv"
	"strings"

	"github.com/nlnwa/whatwg-url/url"
	"golang.org/x/mod/semver"
)

// validateVersion checks the CLI's directory-safe SemVer spelling.
func validateVersion(value Version) error {
	// x/mod requires v and accepts shortened versions. Add v only for checking;
	// canonical equality rejects shorthand and build metadata without rewriting
	// the version carried on the wire or used as a directory name.
	version := "v" + string(value)
	if !semver.IsValid(version) || semver.Canonical(version) != version {
		return errors.New("is not a SemVer version without build metadata")
	}
	core, _, _ := strings.Cut(string(value), "-")
	for component := range strings.SplitSeq(core, ".") {
		// Each of the three numeric components must fit an unsigned 64-bit integer.
		if _, err := strconv.ParseUint(component, 10, 64); err != nil {
			return fmt.Errorf("version component: %w", err)
		}
	}
	return nil
}

// validateArtifact checks the absolute URL without changing its wire spelling.
func validateArtifact(value Artifact) error {
	// WHATWG parsing accepts non-HTTP schemes. The download owner decides
	// which transports it can fetch.
	if _, err := url.Parse(value.URL); err != nil {
		return fmt.Errorf("url: %w", err)
	}
	return nil
}
