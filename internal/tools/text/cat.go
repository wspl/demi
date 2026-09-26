package text

import (
	"io"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/tools/text/cli"
)

const catUsage = `Usage: %[1]s [OPTION]... [FILE]...
Concatenate FILE(s) to standard output.

With no FILE, or when FILE is -, read standard input.

  -A, --show-all           equivalent to -vET
  -b, --number-nonblank    number nonempty output lines, overrides -n
  -e                       equivalent to -vE
  -E, --show-ends          display $ at end of each line
  -n, --number             number all output lines
  -s, --squeeze-blank      suppress repeated empty output lines
  -t                       equivalent to -vT
  -T, --show-tabs          display TAB characters as ^I
  -u                       (ignored)
  -v, --show-nonprinting   use ^ and M- notation, except for LFD and TAB
      --help               display this help and exit
      --version            output version information and exit
`

var catOptions = []cli.Option{
	{Short: 'b', Long: "number-nonblank"},
	{Short: 'n', Long: "number"},
	{Short: 's', Long: "squeeze-blank"},
	{Short: 'v', Long: "show-nonprinting"},
	{Short: 'E', Long: "show-ends"},
	{Short: 'T', Long: "show-tabs"},
	{Short: 'A', Long: "show-all"},
	{Short: 'e'},
	{Short: 't'},
	{Short: 'u'},
}

// catFormat is what cat does to the bytes it copies.
type catFormat struct {
	numberNonblank bool
	number         bool
	squeeze        bool
	nonprinting    bool
	ends           bool
	tabs           bool
}

// plain reports whether the bytes are copied unchanged.
func (f catFormat) plain() bool {
	return !f.numberNonblank && !f.number && !f.squeeze && !f.nonprinting && !f.ends && !f.tabs
}

// catState carries line numbering and blank-line squeezing from one input
// to the next, as cat does.
type catState struct {
	format    catFormat
	line      int
	atStart   bool
	blankRuns int
}

func cat(inv *toolctx.Invocation, args []string) int {
	prog := &cli.Program{Name: args[0], Inv: inv, Usage: catUsage, BadUsage: 1}
	options, operands, status, done := prog.Parse(args[1:], catOptions)
	if done {
		return status
	}
	var format catFormat
	for _, option := range options {
		switch option.Key {
		case "number-nonblank":
			format.numberNonblank = true
		case "number":
			format.number = true
		case "squeeze-blank":
			format.squeeze = true
		case "show-nonprinting":
			format.nonprinting = true
		case "show-ends":
			format.ends = true
		case "show-tabs":
			format.tabs = true
		case "show-all":
			format.nonprinting = true
			format.ends = true
			format.tabs = true
		case "e":
			format.nonprinting = true
			format.ends = true
		case "t":
			format.nonprinting = true
			format.tabs = true
		}
	}
	if format.numberNonblank {
		format.number = false
	}
	if len(operands) == 0 {
		operands = []string{cli.StdinName}
	}
	stdout := cli.Writer(inv.Context, inv.Stdout)
	state := &catState{format: format, atStart: true}
	status = 0
	for _, name := range operands {
		input, err := cli.Open(inv, name)
		if err != nil {
			prog.FileError(name, err)
			status = 1
			continue
		}
		readErr, writeErr := state.copy(stdout, input)
		closeErr := input.Close()
		if writeErr != nil {
			return prog.WriteError(writeErr)
		}
		if readErr == nil {
			readErr = closeErr
		}
		if readErr != nil {
			prog.FileError(name, readErr)
			status = 1
		}
	}
	return status
}

// copy copies one input and returns the read and the write error apart,
// since cat reports them differently.
func (s *catState) copy(stdout io.Writer, input io.Reader) (readErr, writeErr error) {
	buffer := make([]byte, 128*1024)
	out := cli.NewOutput(stdout)
	for {
		n, err := input.Read(buffer)
		if n > 0 {
			if s.format.plain() {
				_, writeErr = stdout.Write(buffer[:n])
			} else {
				s.format.translate(s, out, buffer[:n])
				writeErr = out.Flush()
			}
			if writeErr != nil {
				return nil, writeErr
			}
		}
		if err == io.EOF {
			return nil, nil
		}
		if err != nil {
			return err, nil
		}
	}
}

// translate writes chunk with cat's formatting.
func (f catFormat) translate(s *catState, out *cli.Output, chunk []byte) {
	for _, c := range chunk {
		if c == '\n' {
			if s.atStart {
				s.blankRuns++
				if f.squeeze && s.blankRuns > 1 {
					continue
				}
				if f.number {
					s.line++
					out.Putf("%6d\t", s.line)
				}
			}
			if f.ends {
				out.PutByte('$')
			}
			out.PutByte('\n')
			s.atStart = true
			continue
		}
		if s.atStart {
			s.blankRuns = 0
			if f.number || f.numberNonblank {
				s.line++
				out.Putf("%6d\t", s.line)
			}
			s.atStart = false
		}
		switch {
		case c == '\t':
			if f.tabs {
				out.PutString("^I")
			} else {
				out.PutByte(c)
			}
		case f.nonprinting:
			writeVisible(out, c)
		default:
			out.PutByte(c)
		}
	}
}

// writeVisible writes a byte in cat -v's ^ and M- notation.
func writeVisible(out *cli.Output, c byte) {
	if c >= 128 {
		out.PutString("M-")
		c -= 128
	}
	switch {
	case c < 32:
		out.PutByte('^')
		out.PutByte(c + 64)
	case c == 127:
		out.PutString("^?")
	default:
		out.PutByte(c)
	}
}
