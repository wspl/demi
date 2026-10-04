package tabs

import (
	"encoding/binary"
	"os"
	"runtime"
)

func smeWithoutSVE(hwcap, hwcap2 uint64) bool {
	return hwcap2&(1<<23) != 0 && hwcap&(1<<22) == 0
}

// captureUnavailable prevents Chrome's Linux SME-without-SVE capture crash.
func captureUnavailable() string {
	if runtime.GOOS != "linux" || runtime.GOARCH != "arm64" {
		return ""
	}
	data, err := os.ReadFile("/proc/self/auxv")
	if err != nil {
		return ""
	} // An unreadable auxiliary vector reads as no capabilities, as getauxval reports zero.
	var hwcap, hwcap2 uint64
	for len(data) >= 16 {
		key, value := binary.NativeEndian.Uint64(data), binary.NativeEndian.Uint64(data[8:])
		switch key {
		case 16:
			hwcap = value
		case 26:
			hwcap2 = value
		}
		data = data[16:]
	}
	if smeWithoutSVE(hwcap, hwcap2) {
		return "this Host's CPU reports SME without SVE, which Chrome's tab capture cannot run on; boot its " +
			"kernel with arm64.nosme to show the browser"
	}
	return ""
}
