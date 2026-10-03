package browserop

import (
	_ "embed"
	"fmt"

	"github.com/nlnwa/whatwg-url/url"
)

// Resource is the resource of `demi.browser` that the pinned release is, whose entry
// is Chrome's executable (`native-runtime.md` § Bind an exact package).
const Resource = "chrome"

//go:embed chrome.json
var pinnedRelease []byte

// PinnedRelease returns the release this build of Demi pins.
func PinnedRelease() (BrowserRelease, error) { return DecodeBrowserRelease(pinnedRelease) }

// Title returns the resource's title for the user.
func (r BrowserRelease) Title() string { return "Chrome for Testing " + r.Version }

// Platform returns the archive for target, when the release has one.
func (r BrowserRelease) Platform(target string) *ReleasePlatform {
	for i := range r.Platforms {
		if r.Platforms[i].Target == target {
			return &r.Platforms[i]
		}
	}
	return nil
}

func validateReleasePlatform(p ReleasePlatform) error {
	_, err := url.Parse(p.URL)
	if err != nil {
		return fmt.Errorf("url: %w", err)
	}
	return nil
}
