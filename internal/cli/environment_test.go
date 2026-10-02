package cli_test

import (
	"testing"

	"github.com/wspl/demi/internal/cli"
)

// Pure validation: no IO, processes, or waits; the table takes under one second.
func TestUnknownVariable(t *testing.T) {
	tests := []struct {
		name    string
		prefix  string
		known   []string
		environ []string
		want    string
	}{
		{
			name:   "empty environment",
			prefix: "DEMI_",
		},
		{
			name:    "declared settings and unrelated variables",
			prefix:  "DEMI_",
			known:   []string{"DEMI_PORT", "DEMI_URL"},
			environ: []string{"PATH=/bin", "DEMI_PORT=", "DEMI_URL=https://example.com/?a=b", "OTHER=DEMI_UNKNOWN"},
		},
		{
			name:    "unknown backend setting",
			prefix:  "DEMI_",
			known:   []string{"DEMI_PORT"},
			environ: []string{"DEMI_PORRT=secret"},
			want:    "unknown environment variable: DEMI_PORRT",
		},
		{
			name:    "obsolete managed setting is named",
			prefix:  "DEMI_MANAGED_",
			known:   []string{"DEMI_MANAGED_RUNSC"},
			environ: []string{"DEMI_MANAGED_RUNSC=/opt/gvisor/runsc", "DEMI_MANAGED_FIRECRACKER=/old"},
			want:    "unknown environment variable: DEMI_MANAGED_FIRECRACKER",
		},
		{
			name:    "manager ignores other prefixes",
			prefix:  "DEMI_MANAGED_",
			environ: []string{"DEMI_PORT=80", "DEMI_MACHINE_MANAGER_SOCKET=/run/socket", "OTHER_DEMI_MANAGED_UNKNOWN=1", "demi_managed_unknown=1"},
		},
		{
			name:    "first unknown in environment order",
			prefix:  "DEMI_",
			environ: []string{"DEMI_Z=1", "DEMI_A=2"},
			want:    "unknown environment variable: DEMI_Z",
		},
		{
			name:    "known names must match exactly",
			prefix:  "DEMI_",
			known:   []string{"DEMI_PORT", "DEMI_MODE"},
			environ: []string{"DEMI_PORT_EXTRA=1"},
			want:    "unknown environment variable: DEMI_PORT_EXTRA",
		},
		{
			name:    "known names are case sensitive",
			prefix:  "DEMI_",
			known:   []string{"DEMI_PORT"},
			environ: []string{"DEMI_port=1"},
			want:    "unknown environment variable: DEMI_port",
		},
		{
			name:    "invalid Unicode names ignored and values opaque",
			prefix:  "DEMI_",
			known:   []string{"DEMI_PORT"},
			environ: []string{"DEMI_\xff=1", "DEMI_PORT=\xff", "DEMI_未知=1"},
			want:    "unknown environment variable: DEMI_未知",
		},
		{
			name:    "empty prefix checks all names",
			environ: []string{"PATH=/bin"},
			want:    "unknown environment variable: PATH",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := cli.UnknownVariable(tt.prefix, tt.known, tt.environ)
			if tt.want == "" {
				if err != nil {
					t.Fatalf("UnknownVariable() = %v, want nil", err)
				}
				return
			}
			if err == nil || err.Error() != tt.want {
				t.Fatalf("UnknownVariable() = %v, want %q", err, tt.want)
			}
		})
	}
}
