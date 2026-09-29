package rustfmt

import (
	"math"
	"strings"
	"testing"
)

func TestJSONFloatFollowsSerdeJSON(t *testing.T) {
	for _, tc := range []struct {
		in   float64
		want string
	}{
		{0, "0.0"}, {1, "1.0"}, {2, "2.0"}, {-3, "-3.0"}, {0.5, "0.5"}, {360, "360.0"},
		{1.25, "1.25"}, {1280, "1280.0"}, {0.0001, "0.0001"}, {0.00001, "0.00001"}, {0.000001, "1e-6"},
		{123456789012345, "123456789012345.0"}, {1e15, "1000000000000000.0"},
		{1e16, "1e16"}, {1e21, "1e21"}, {1.5e300, "1.5e300"}, {1234e30, "1.234e33"},
		{1.5e-7, "1.5e-7"}, {0.001234, "0.001234"}, {12.34, "12.34"},
		{1.7976931348623157e308, "1.7976931348623157e308"},
	} {
		if got := JSONFloat(tc.in); got != tc.want {
			t.Errorf("JSONFloat(%v) = %q, want %q", tc.in, got, tc.want)
		}
	}
}

func TestDisplayFloatIsPlain(t *testing.T) {
	for in, want := range map[float64]string{1: "1", 360: "360", 0.5: "0.5", 1e21: "1000000000000000000000", 1.5e-7: "0.00000015", -0.25: "-0.25"} {
		if got := DisplayFloat(in); got != want {
			t.Errorf("DisplayFloat(%v) = %q, want %q", in, got, want)
		}
	}
}

func TestQuoteFollowsDebug(t *testing.T) {
	for in, want := range map[string]string{
		"Delete this record?": `"Delete this record?"`,
		"a\"b\\c\nd\te\rf":    `"a\"b\\c\nd\te\rf"`,
		"it's":                `"it's"`,
		"\x1b[31m":            `"\u{1b}[31m"`,
		"nul\x00":             `"nul\0"`,
		"é":                  `"e\u{301}"`,
		"\u200b":              `"\u{200b}"`,
		"浏览器 é €":             `"浏览器 é €"`,
	} {
		if got := Quote(in); got != want {
			t.Errorf("Quote(%q) = %s, want %s", in, got, want)
		}
	}
}

func TestParseFloatFollowsRust(t *testing.T) {
	for in, want := range map[string]float64{"12": 12, "1.": 1, ".5": 0.5, "+3.5": 3.5, "-0": 0, "1e3": 1000, "2E-2": 0.02, "1e999": math.Inf(1)} {
		if got, ok := ParseFloat(in); !ok || got != want {
			t.Errorf("ParseFloat(%q) = %v, %v, want %v", in, got, ok, want)
		}
	}
	for _, in := range []string{"", " 1", "1 ", "0x1p3", "1_000", "e5", ".", "--1", "1e", "١٢"} {
		if _, ok := ParseFloat(in); ok {
			t.Errorf("ParseFloat(%q) was accepted", in)
		}
	}
	if got, ok := ParseFloat("-Infinity"); !ok || !math.IsInf(got, -1) {
		t.Errorf("ParseFloat(-Infinity) = %v, %v", got, ok)
	}
	if got, ok := ParseFloat("NaN"); !ok || !math.IsNaN(got) {
		t.Errorf("ParseFloat(NaN) = %v, %v", got, ok)
	}
}

func TestNormalizeJSONFollowsSerde(t *testing.T) {
	for in, want := range map[string]string{
		`{"b":1,"a":[1.0,2e0,-3,1e+21,12345678901234567890,123456789012345678901]}`: `{"b":1,"a":[1.0,2.0,-3,1e21,12345678901234567890,1.2345678901234568e20]}`,
		`{"k":1,"j":2,"k":3}`:            `{"k":3,"j":2}`,
		"\"a\\u003cb\\u0007\\u007f\\/\"": `"a<b\u0007` + "\x7f" + `/"`,
		` [ true , null , "é" ] `:        `[true,null,"é"]`,
		`{}`:                             `{}`,
	} {
		got, err := NormalizeJSON([]byte(in))
		if err != nil || string(got) != want {
			t.Errorf("NormalizeJSON(%s) = %s, %v, want %s", in, got, err, want)
		}
	}
	for _, in := range []string{``, `{`, `1e999`, `"\ud800"`, `[] 1`, strings.Repeat("[", 128) + strings.Repeat("]", 128)} {
		if got, err := NormalizeJSON([]byte(in)); err == nil {
			t.Errorf("NormalizeJSON(%q) = %s, want an error", in, got)
		}
	}
	deep := strings.Repeat("[", 127) + strings.Repeat("]", 127)
	if got, err := NormalizeJSON([]byte(deep)); err != nil || string(got) != deep {
		t.Errorf("127 levels: %v", err)
	}
}
