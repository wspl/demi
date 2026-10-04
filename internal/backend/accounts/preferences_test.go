package accounts

import (
	"os"
	"strings"
	"testing"
)

func TestLocaleFixtures(t *testing.T) {
	data, err := os.ReadFile("testdata/locales.tsv")
	if err != nil {
		t.Fatal(err)
	}
	for _, row := range strings.Split(strings.TrimSpace(string(data)), "\n") {
		fields := strings.Split(row, "\t")
		t.Run(fields[0], func(t *testing.T) {
			got, err := canonicalLanguage(fields[0])
			if fields[1] == "ERROR" {
				if err == nil {
					t.Fatalf("the fixture refuses %q, canonicalLanguage accepted it as %q", fields[0], got)
				}
			} else if err != nil || got != fields[1] {
				t.Fatalf("got %q (%v), the fixture wants %q", got, err, fields[1])
			}
		})
	}
}
