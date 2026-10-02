package contract_test

import (
	"math"
	"testing"

	"github.com/wspl/demi/internal/contract"
)

// Serde accepts signed/unsigned integers for floats without routing u64 through
// i64. Local boundary calls only; budget below one second, no IO or waits.
func TestMsgpackFloatNumbers(t *testing.T) {
	for _, test := range []struct {
		data []byte
		want float64
	}{
		{[]byte{0xff}, -1},
		{[]byte{0xcf, 255, 255, 255, 255, 255, 255, 255, 255}, float64(math.MaxUint64)},
		{[]byte{0xd3, 128, 0, 0, 0, 0, 0, 0, 0}, float64(math.MinInt64)},
		{[]byte{0xca, 0x3f, 0xc0, 0, 0}, 1.5},
		{[]byte{0xcb, 0x3f, 0xf8, 0, 0, 0, 0, 0, 0}, 1.5},
	} {
		got, err := contract.DecodeMsgpack[float64](test.data)
		if err != nil || got != test.want {
			t.Fatalf("float64 %x: got %v %v, want %v", test.data, got, err, test.want)
		}
		f32, err := contract.DecodeMsgpack[float32](test.data)
		if err != nil || f32 != float32(test.want) {
			t.Fatalf("float32 %x: got %v %v, want %v", test.data, f32, err, float32(test.want))
		}
	}
	for _, data := range [][]byte{{0xc0}, {0xc3}, {0xa1, '1'}, {0x90}, {0x01, 0x02}, {0xcf}, {0xcb}} {
		if _, err := contract.DecodeMsgpack[float64](data); err == nil {
			t.Fatalf("accepted non-number or malformed token %x", data)
		}
	}
}
