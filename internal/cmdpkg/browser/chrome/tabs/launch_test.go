package tabs

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"

	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

func TestCaptureExtensionIDIsItsManifestKey(t *testing.T) {
	data, err := captureExtension.ReadFile("extension/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	var manifest struct {
		Key string `json:"key"`
	}
	if err := json.Unmarshal(data, &manifest); err != nil {
		t.Fatal(err)
	}
	key, err := base64.StdEncoding.DecodeString(manifest.Key)
	if err != nil {
		t.Fatal(err)
	}
	hash := sha256.Sum256(key)
	id := make([]byte, 0, 32)
	for _, value := range hash[:16] {
		id = append(id, 'a'+(value>>4), 'a'+(value&15))
	}
	if string(id) != captureExtensionID {
		t.Fatalf("manifest ID %q, want %q", id, captureExtensionID)
	}
}

func TestUserAgentCarriesMajorVersionWithoutHeadlessToken(t *testing.T) {
	agent := desktopUserAgent("153.0.8010.36")
	if !strings.Contains(agent, "Chrome/153.0.0.0") || strings.Contains(agent, "Headless") {
		t.Fatal(agent)
	}
}

func TestCPUWithSMEWithoutSVECannotCapture(t *testing.T) {
	hwcap := uint64(0xefffffff) &^ (1 << 22)
	if !smeWithoutSVE(hwcap, 0x1a03fb26181) || smeWithoutSVE(hwcap, 0x326181) || smeWithoutSVE(hwcap|1<<22, 0x1a03fb26181) {
		t.Fatal("capture capability mismatch")
	}
}
