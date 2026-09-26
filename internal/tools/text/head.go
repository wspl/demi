package text

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"strings"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/tools/text/cli"
)

const headUsage = `Usage: %[1]s [OPTION]... [FILE]...
Print the first 10 lines of each FILE to standard output.
With more than one FILE, precede each with a header giving the file name.

With no FILE, or when FILE is -, read standard input.

  -c, --bytes=[-]NUM       print the first NUM bytes of each file;
                             with the leading '-', print all but the last
                             NUM bytes of each file
  -n, --lines=[-]NUM       print the first NUM lines instead of the first 10;
                             with the leading '-', print all but the last
                             NUM lines of each file
  -q, --quiet, --silent    never print headers giving file names
  -v, --verbose            always print headers giving file names
  -z, --zero-terminated    line delimiter is NUL, not newline
      --help               display this help and exit
      --version            output version information and exit

NUM may have a multiplier suffix: b 512, kB 1000, K 1024, MB 1000*1000,
M 1024*1024, GB 1000*1000*1000, G 1024*1024*1024, and so on for T, P, E, Z,
Y, R, Q. Binary prefixes can be used, too: KiB=K, MiB=M, and so on.
`

var headOptions = []cli.Option{
	{Short: 'c', Long: "bytes", Arg: cli.RequiredArg},
	{Short: 'n', Long: "lines", Arg: cli.RequiredArg},
	{Short: 'q', Long: "quiet"},
	{Long: "silent", Key: "quiet"},
	{Short: 'v', Long: "verbose"},
	{Short: 'z', Long: "zero-terminated"},
}

func init() {
	// head accepts digits as options only to reject them after the first
	// argument; the first argument may be the obsolete -NUM form.
	for digit := '0'; digit <= '9'; digit++ {
		headOptions = append(headOptions, cli.Option{Short: digit, Key: "digit"})
	}
}

// headers says when head and tail print a header before each file.
type headers int

const (
	headersAuto headers = iota
	headersNever
	headersAlways
)

// countSpec is the part head prints: the first count lines or bytes, or
// all but the last count.
type countSpec struct {
	bytes      bool
	count      uint64
	allButLast bool
}

func head(inv *toolctx.Invocation, args []string) int {
	prog := &cli.Program{Name: args[0], Inv: inv, Usage: headUsage, BadUsage: 1}
	spec := countSpec{count: 10}
	show := headersAuto
	delimiter := byte('\n')
	rest := args[1:]
	if len(rest) > 0 && isObsoleteHeadCount(rest[0]) {
		var err error
		spec, show, delimiter, err = parseObsoleteHead(rest[0])
		if err != nil {
			var usage *cli.UsageError
			if errors.As(err, &usage) {
				return prog.UsageError(err)
			}
			prog.Errorf("%s", err)
			return 1
		}
		rest = rest[1:]
	}
	options, operands, status, done := prog.Parse(rest, headOptions)
	if done {
		return status
	}
	for _, option := range options {
		switch option.Key {
		case "bytes", "lines":
			value := option.Value
			spec = countSpec{bytes: option.Key == "bytes"}
			if strings.HasPrefix(value, "-") {
				spec.allButLast = true
				value = value[1:]
			}
			count, err := cli.ParseUint(value, cli.SizeSuffixes)
			if err != nil {
				prog.Errorf("%s", countError(spec.bytes, value, err))
				return 1
			}
			spec.count = count
		case "quiet":
			show = headersNever
		case "verbose":
			show = headersAlways
		case "zero-terminated":
			delimiter = 0
		case "digit":
			prog.Errorf("invalid trailing option -- %s", strings.TrimPrefix(option.Name, "-"))
			prog.TryHelp()
			return 1
		}
	}
	if len(operands) == 0 {
		operands = []string{cli.StdinName}
	}
	printHeaders := show == headersAlways || (show == headersAuto && len(operands) > 1)
	stdout := cli.Writer(inv.Context, inv.Stdout)
	first := true
	status = 0
	for _, name := range operands {
		input, err := cli.Open(inv, name)
		if err != nil {
			prog.Errorf("cannot open %s for reading: %s", cli.Quote(name), cli.Strerror(err))
			status = 1
			continue
		}
		if printHeaders {
			if _, err := fmt.Fprintf(stdout, "%s==> %s <==\n", separator(first), displayName(name)); err != nil {
				input.Release()
				return prog.WriteError(err)
			}
			first = false
		}
		readErr, writeErr := headCopy(stdout, input, spec, delimiter)
		closeErr := input.Close()
		if writeErr != nil {
			return prog.WriteError(writeErr)
		}
		if readErr == nil {
			readErr = closeErr
		}
		if readErr != nil {
			prog.Errorf("error reading %s: %s", cli.Quote(name), cli.Strerror(readErr))
			status = 1
		}
	}
	return status
}

// separator is the empty line head and tail print before every header but
// the first.
func separator(first bool) string {
	if first {
		return ""
	}
	return "\n"
}

// displayName is the name a header shows for an operand.
func displayName(name string) string {
	if name == cli.StdinName {
		return "standard input"
	}
	return name
}

// countError is the message for a bad count argument of head and tail.
func countError(bytes bool, value string, err error) string {
	unit := "lines"
	if bytes {
		unit = "bytes"
	}
	message := fmt.Sprintf("invalid number of %s: %s", unit, cli.Quote(value))
	if errors.Is(err, cli.ErrOverflow) {
		message += ": " + cli.Strerror(err)
	}
	return message
}

// isObsoleteHeadCount reports whether arg is head's obsolete -NUM option.
func isObsoleteHeadCount(arg string) bool {
	return len(arg) > 1 && arg[0] == '-' && arg[1] >= '0' && arg[1] <= '9'
}

// parseObsoleteHead parses -NUM[bcklmqvz]..., the obsolete first argument
// of head. Each unit letter sets the unit: b, k and m count blocks of 512,
// 1024 and 1048576 bytes, c bytes and l lines; the last one wins.
func parseObsoleteHead(arg string) (countSpec, headers, byte, error) {
	body := arg[1:]
	digits := 0
	for digits < len(body) && body[digits] >= '0' && body[digits] <= '9' {
		digits++
	}
	spec := countSpec{}
	show := headersAuto
	delimiter := byte('\n')
	multiplier := uint64(1)
	for _, flag := range body[digits:] {
		switch flag {
		case 'b', 'k', 'm':
			spec.bytes = true
			multiplier = map[rune]uint64{'b': 512, 'k': 1024, 'm': 1024 * 1024}[flag]
		case 'c':
			spec.bytes = true
			multiplier = 1
		case 'l':
			spec.bytes = false
			multiplier = 1
		case 'q':
			show = headersNever
		case 'v':
			show = headersAlways
		case 'z':
			delimiter = 0
		default:
			return spec, show, delimiter, &cli.UsageError{Message: fmt.Sprintf("invalid trailing option -- %c", flag)}
		}
	}
	count, err := cli.ParseUint(body[:digits], "")
	if err == nil && count > 0 && multiplier > ^uint64(0)/count {
		err = cli.ErrOverflow
	}
	if err != nil {
		return spec, show, delimiter, errors.New(countError(spec.bytes, body[:digits], err))
	}
	spec.count = count * multiplier
	return spec, show, delimiter, nil
}

// headCopy copies the part of one input that spec selects. It returns the
// read and the write error apart.
func headCopy(stdout io.Writer, input io.Reader, spec countSpec, delimiter byte) (readErr, writeErr error) {
	switch {
	case spec.bytes && !spec.allButLast:
		return copyN(stdout, input, spec.count)
	case !spec.bytes && !spec.allButLast:
		return copyLines(stdout, input, spec.count, delimiter)
	case spec.bytes:
		return copyAllButLastBytes(stdout, input, spec.count)
	default:
		return copyAllButLastLines(stdout, input, spec.count, delimiter)
	}
}

// copyN copies the first n bytes.
func copyN(stdout io.Writer, input io.Reader, n uint64) (readErr, writeErr error) {
	buffer := make([]byte, 64*1024)
	for n > 0 {
		size := uint64(len(buffer))
		if n < size {
			size = n
		}
		read, err := input.Read(buffer[:size])
		if read > 0 {
			if _, err := stdout.Write(buffer[:read]); err != nil {
				return nil, err
			}
			n -= uint64(read)
		}
		if err == io.EOF {
			return nil, nil
		}
		if err != nil {
			return err, nil
		}
	}
	return nil, nil
}

// copyLines copies the first n lines.
func copyLines(stdout io.Writer, input io.Reader, n uint64, delimiter byte) (readErr, writeErr error) {
	buffer := make([]byte, 64*1024)
	for n > 0 {
		read, err := input.Read(buffer)
		chunk := buffer[:read]
		end := len(chunk)
		for position := 0; position < len(chunk); {
			found := bytes.IndexByte(chunk[position:], delimiter)
			if found < 0 {
				break
			}
			position += found + 1
			n--
			if n == 0 {
				end = position
				break
			}
		}
		if end > 0 {
			if _, err := stdout.Write(chunk[:end]); err != nil {
				return nil, err
			}
		}
		if err == io.EOF {
			return nil, nil
		}
		if err != nil {
			return err, nil
		}
	}
	return nil, nil
}

// copyAllButLastBytes copies everything but the last n bytes, holding back
// only those n bytes.
func copyAllButLastBytes(stdout io.Writer, input io.Reader, n uint64) (readErr, writeErr error) {
	var held []byte
	buffer := make([]byte, 64*1024)
	for {
		read, err := input.Read(buffer)
		held = append(held, buffer[:read]...)
		if excess := uint64(len(held)); excess > n {
			if _, err := stdout.Write(held[:excess-n]); err != nil {
				return nil, err
			}
			held = append(held[:0], held[excess-n:]...)
		}
		if err == io.EOF {
			return nil, nil
		}
		if err != nil {
			return err, nil
		}
	}
}

// copyAllButLastLines copies everything but the last n lines, holding back
// only those lines. A final line without a delimiter counts as a line.
func copyAllButLastLines(stdout io.Writer, input io.Reader, n uint64, delimiter byte) (readErr, writeErr error) {
	var held []byte
	// ends are the offsets in held just after each complete line.
	var ends []int
	buffer := make([]byte, 64*1024)
	for {
		read, err := input.Read(buffer)
		start := len(held)
		held = append(held, buffer[:read]...)
		for position := start; ; {
			found := bytes.IndexByte(held[position:], delimiter)
			if found < 0 {
				break
			}
			position += found + 1
			ends = append(ends, position)
		}
		if err != nil && err != io.EOF {
			return err, nil
		}
		complete := uint64(len(ends))
		if err == io.EOF && len(held) > 0 && (len(ends) == 0 || ends[len(ends)-1] != len(held)) {
			// The unterminated last line is one of the lines held back.
			complete++
			ends = append(ends, len(held))
		}
		if complete > n {
			release := ends[complete-n-1]
			if _, err := stdout.Write(held[:release]); err != nil {
				return nil, err
			}
			held = append(held[:0], held[release:]...)
			ends = ends[complete-n:]
			for index := range ends {
				ends[index] -= release
			}
		}
		if err == io.EOF {
			return nil, nil
		}
	}
}
