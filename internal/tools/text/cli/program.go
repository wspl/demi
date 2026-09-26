package cli

import (
	"errors"
	"fmt"
	"io"
	"io/fs"
	"strings"
	"syscall"

	"github.com/wspl/demi/internal/toolctx"
)

// Program is one run of a utility: its name for diagnostics, its streams and
// its command-line conventions.
type Program struct {
	// Name is the name the utility was called by; every diagnostic starts
	// with it.
	Name string
	Inv  *toolctx.Invocation
	// Usage is the --help text. "%[1]s" stands for Name.
	Usage string
	// BadUsage is the exit status for an invalid command line.
	BadUsage int
	// OptionsFirst ends the options at the first operand, for utilities
	// whose operands may start with "-".
	OptionsFirst bool
}

// helpOptions are the options every utility accepts.
var helpOptions = []Option{
	{Long: "help"},
	{Long: "version"},
}

// Parse parses the command line as Parse does and handles --help, --version
// and command-line errors. It returns the options and the operands apart.
// When done is true the utility returns status at once. Like getopt_long,
// --help and --version act when parsing reaches them, so an error after
// them is not reported.
func (p *Program) Parse(args []string, options []Option) (parsed []Parsed, operands []string, status int, done bool) {
	all, status, done := p.ParseInOrder(args, options)
	if done {
		return nil, nil, status, true
	}
	for _, item := range all {
		if item.Key == Operand {
			operands = append(operands, item.Value)
		} else {
			parsed = append(parsed, item)
		}
	}
	return parsed, operands, 0, false
}

// ParseInOrder is Parse with options and operands in one list in
// command-line order, for utilities whose operands depend on position.
func (p *Program) ParseInOrder(args []string, options []Option) (parsed []Parsed, status int, done bool) {
	all := append(append([]Option{}, options...), helpOptions...)
	parsed, err := Parse(args, all, p.OptionsFirst, p.Inv.Env)
	for _, option := range parsed {
		switch option.Key {
		case "help":
			return nil, p.Help(), true
		case "version":
			return nil, p.Version(), true
		}
	}
	if err != nil {
		return nil, p.UsageError(err), true
	}
	return parsed, 0, false
}

// Help prints the usage text to standard output.
func (p *Program) Help() int {
	_, err := fmt.Fprintf(p.Inv.Stdout, p.Usage, p.Name)
	if err != nil {
		return p.WriteError(err)
	}
	return 0
}

// Version prints the version to standard output.
func (p *Program) Version() int {
	_, err := fmt.Fprintf(p.Inv.Stdout, "%s (Demi)\n", p.Name)
	if err != nil {
		return p.WriteError(err)
	}
	return 0
}

// Errorf prints "NAME: message".
func (p *Program) Errorf(format string, args ...any) {
	// A diagnostic that cannot be written has nowhere else to go.
	_, _ = fmt.Fprintf(p.Inv.Stderr, "%s: %s\n", p.Name, fmt.Sprintf(format, args...))
}

// FileError prints "NAME: FILE: reason" with FILE quoted as coreutils quotes
// file names.
func (p *Program) FileError(name string, err error) {
	p.Errorf("%s: %s", QuoteName(name), Strerror(err))
}

// UsageError reports an invalid command line followed by the hint to run
// --help, and returns the exit status for it.
func (p *Program) UsageError(err error) int {
	p.Errorf("%s", err.Error())
	p.TryHelp()
	var usage *UsageError
	if errors.As(err, &usage) && usage.Status != 0 {
		return usage.Status
	}
	return p.BadUsage
}

// TryHelp prints the hint that follows a command-line error.
func (p *Program) TryHelp() {
	// See Errorf.
	_, _ = fmt.Fprintf(p.Inv.Stderr, "Try '%s --help' for more information.\n", p.Name)
}

// StatusBrokenPipe is the status of a GNU utility killed by SIGPIPE, as a
// shell reports it.
const StatusBrokenPipe = 128 + int(syscall.SIGPIPE)

// WriteError handles a failed write to standard output and returns the exit
// status. A write that fails because the job was cancelled or because the
// reader went away ends the utility silently, as SIGPIPE ends a GNU utility;
// any other failure is reported as GNU reports it.
func (p *Program) WriteError(err error) int {
	if p.Inv.Context.Err() != nil {
		return 1
	}
	if IsBrokenPipe(err) {
		return StatusBrokenPipe
	}
	p.Errorf("write error: %s", Strerror(err))
	return 1
}

// IsBrokenPipe reports whether a write failed because its reader went away.
func IsBrokenPipe(err error) bool {
	return errors.Is(err, syscall.EPIPE) || errors.Is(err, io.ErrClosedPipe)
}

// Strerror returns the C library's message for the error's errno, as GNU
// utilities print it.
func Strerror(err error) string {
	var errno syscall.Errno
	if errors.As(err, &errno) {
		return capitalize(errno.Error())
	}
	// A Files implementation may report a condition without an errno.
	for _, known := range []struct {
		err   error
		errno syscall.Errno
	}{
		{fs.ErrNotExist, syscall.ENOENT},
		{fs.ErrPermission, syscall.EACCES},
		{fs.ErrExist, syscall.EEXIST},
	} {
		if errors.Is(err, known.err) {
			return capitalize(known.errno.Error())
		}
	}
	var pathErr *fs.PathError
	if errors.As(err, &pathErr) {
		return pathErr.Err.Error()
	}
	return err.Error()
}

// capitalize turns Go's errno text into glibc's, which differs only in the
// first letter.
func capitalize(message string) string {
	if message == "" || message[0] < 'a' || message[0] > 'z' {
		return message
	}
	return strings.ToUpper(message[:1]) + message[1:]
}
