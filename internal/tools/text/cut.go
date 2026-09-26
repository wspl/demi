package text

import (
	"bufio"
	"bytes"
	"errors"
	"fmt"
	"io"
	"math"
	"sort"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/tools/text/cli"
)

const cutUsage = `Usage: %[1]s OPTION... [FILE]...
Print selected parts of lines from each FILE to standard output.

With no FILE, or when FILE is -, read standard input.

  -b, --bytes=LIST        select only these bytes
  -c, --characters=LIST   select only these characters
  -d, --delimiter=DELIM   use DELIM instead of TAB for field delimiter
  -f, --fields=LIST       select only these fields;  also print any line
                            that contains no delimiter character, unless
                            the -s option is specified
  -n                      (ignored)
      --complement        complement the set of selected bytes, characters
                            or fields
  -s, --only-delimited    do not print lines not containing delimiters
      --output-delimiter=STRING  use STRING as the output delimiter
                            the default is to use the input delimiter
  -z, --zero-terminated   line delimiter is NUL, not newline
      --help              display this help and exit
      --version           output version information and exit

Use one, and only one of -b, -c or -f.  Each LIST is made up of one
range, or many ranges separated by commas.  Selected input is written
in the same order that it is read, and is written exactly once.
Each range is one of:

  N     N'th byte, character or field, counted from 1
  N-    from N'th byte, character or field, to end of line
  N-M   from N'th to M'th (included) byte, character or field
  -M    from first to M'th (included) byte, character or field
`

var cutOptions = []cli.Option{
	{Short: 'b', Long: "bytes", Arg: cli.RequiredArg},
	{Short: 'c', Long: "characters", Arg: cli.RequiredArg},
	{Short: 'd', Long: "delimiter", Arg: cli.RequiredArg},
	{Short: 'f', Long: "fields", Arg: cli.RequiredArg},
	{Short: 'n', Key: "n"},
	{Long: "complement"},
	{Short: 's', Long: "only-delimited"},
	{Long: "output-delimiter", Arg: cli.RequiredArg},
	{Short: 'z', Long: "zero-terminated"},
}

// cutRange is an inclusive range of 1-based positions.
type cutRange struct {
	low, high uint64
}

// cutConfig is a parsed cut command line.
type cutConfig struct {
	fields bool
	ranges []cutRange
	// delimiter is the field delimiter.
	delimiter byte
	// outputDelimiter is set by --output-delimiter or, for fields, is the
	// field delimiter.
	outputDelimiter    []byte
	hasOutputDelimiter bool
	onlyDelimited      bool
	lineEnd            byte
}

func cut(inv *toolctx.Invocation, args []string) int {
	prog := &cli.Program{Name: args[0], Inv: inv, Usage: cutUsage, BadUsage: 1}
	options, operands, status, done := prog.Parse(args[1:], cutOptions)
	if done {
		return status
	}
	config := cutConfig{delimiter: '\t', lineEnd: '\n'}
	list := ""
	hasList := false
	hasDelimiter := false
	complement := false
	for _, option := range options {
		switch option.Key {
		case "bytes", "characters", "fields":
			if hasList {
				return prog.UsageError(&cli.UsageError{Message: "only one list may be specified"})
			}
			hasList = true
			list = option.Value
			config.fields = option.Key == "fields"
		case "delimiter":
			if len(option.Value) > 1 {
				return prog.UsageError(&cli.UsageError{Message: "the delimiter must be a single character"})
			}
			config.delimiter = 0
			if option.Value != "" {
				config.delimiter = option.Value[0]
			}
			hasDelimiter = true
		case "complement":
			complement = true
		case "only-delimited":
			config.onlyDelimited = true
		case "output-delimiter":
			// Like GNU cut, an empty output delimiter is a NUL byte.
			config.outputDelimiter = []byte(option.Value)
			if option.Value == "" {
				config.outputDelimiter = []byte{0}
			}
			config.hasOutputDelimiter = true
		case "zero-terminated":
			config.lineEnd = 0
		}
	}
	if !hasList {
		return prog.UsageError(&cli.UsageError{Message: "you must specify a list of bytes, characters, or fields"})
	}
	if !config.fields && hasDelimiter {
		return prog.UsageError(&cli.UsageError{Message: "an input delimiter may be specified only when operating on fields"})
	}
	if !config.fields && config.onlyDelimited {
		return prog.UsageError(&cli.UsageError{Message: "suppressing non-delimited lines makes sense\n\tonly when operating on fields"})
	}
	ranges, err := parseCutList(list, config.fields)
	if err != nil {
		return prog.UsageError(err)
	}
	if complement {
		ranges = complementRanges(ranges)
	}
	config.ranges = ranges
	if config.fields && !config.hasOutputDelimiter {
		config.outputDelimiter = []byte{config.delimiter}
	}
	if len(operands) == 0 {
		operands = []string{cli.StdinName}
	}
	out := cli.NewOutput(cli.Writer(inv.Context, inv.Stdout))
	status = 0
	for _, name := range operands {
		input, err := cli.Open(inv, name)
		if err != nil {
			prog.FileError(name, err)
			status = 1
			continue
		}
		readErr, writeErr := config.cutInput(input, out)
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
	if err := out.Flush(); err != nil {
		return prog.WriteError(err)
	}
	return status
}

// parseCutList parses a LIST of ranges, as GNU cut reads it character by
// character, and returns the ranges sorted with overlapping ones merged.
func parseCutList(list string, fields bool) ([]cutRange, error) {
	messages := map[bool][4]string{
		false: {"byte/character positions are numbered from 1", "invalid byte or character range", "invalid byte/character position %s", "byte/character offset %s is too large"},
		true:  {"fields are numbered from 1", "invalid field range", "invalid field value %s", "field number %s is too large"},
	}[fields]
	fail := func(format string, args ...any) error {
		return &cli.UsageError{Message: fmt.Sprintf(format, args...)}
	}
	var ranges []cutRange
	var number, low uint64
	hasNumber := false
	hasLow := false
	dash := false
	numberStart := 0
	for index := 0; index <= len(list); index++ {
		var c byte
		if index < len(list) {
			c = list[index]
		}
		switch {
		case index < len(list) && c == '-':
			if dash {
				return nil, fail("%s", messages[1])
			}
			dash = true
			low = 1
			if hasNumber {
				if number == 0 {
					return nil, fail("%s", messages[0])
				}
				low = number
				hasLow = true
			}
			number = 0
			hasNumber = false
		case index == len(list) || c == ',' || c == ' ' || c == '\t':
			if dash {
				high := uint64(math.MaxUint64)
				switch {
				case hasNumber:
					high = number
				case !hasLow:
					return nil, fail("invalid range with no endpoint: -")
				}
				if low > high {
					return nil, fail("invalid decreasing range")
				}
				ranges = append(ranges, cutRange{low, high})
			} else {
				if !hasNumber || number == 0 {
					return nil, fail("%s", messages[0])
				}
				ranges = append(ranges, cutRange{number, number})
			}
			number = 0
			low = 0
			hasNumber = false
			hasLow = false
			dash = false
		case c >= '0' && c <= '9':
			if !hasNumber {
				numberStart = index
			}
			hasNumber = true
			if number > (math.MaxUint64-2-uint64(c-'0'))/10 {
				end := index
				for end < len(list) && list[end] >= '0' && list[end] <= '9' {
					end++
				}
				return nil, fail(messages[3], cli.Quote(list[numberStart:end]))
			}
			number = number*10 + uint64(c-'0')
		default:
			return nil, fail(messages[2], cli.Quote(list[index:]))
		}
	}
	sort.Slice(ranges, func(a, b int) bool { return ranges[a].low < ranges[b].low })
	merged := ranges[:1]
	for _, next := range ranges[1:] {
		last := &merged[len(merged)-1]
		if next.low <= last.high {
			last.high = max(last.high, next.high)
			continue
		}
		merged = append(merged, next)
	}
	return merged, nil
}

// complementRanges returns the positions the sorted ranges leave out.
func complementRanges(ranges []cutRange) []cutRange {
	var gaps []cutRange
	next := uint64(1)
	for _, r := range ranges {
		if r.low > next {
			gaps = append(gaps, cutRange{next, r.low - 1})
		}
		if r.high == math.MaxUint64 {
			return gaps
		}
		next = r.high + 1
	}
	return append(gaps, cutRange{next, math.MaxUint64})
}

// selected reports whether position is in a range, and whether it starts
// one.
func (c *cutConfig) selected(position uint64) (in, start bool) {
	for _, r := range c.ranges {
		if position < r.low {
			return false, false
		}
		if position <= r.high {
			return true, position == r.low
		}
	}
	return false, false
}

// cutInput cuts every line of one input.
func (c *cutConfig) cutInput(input io.Reader, out *cli.Output) (readErr, writeErr error) {
	reader := bufio.NewReaderSize(input, 64*1024)
	for {
		line, err := reader.ReadSlice(c.lineEnd)
		if errors.Is(err, bufio.ErrBufferFull) {
			var rest []byte
			rest, err = reader.ReadBytes(c.lineEnd)
			line = append(append([]byte{}, line...), rest...)
		}
		if err != nil && err != io.EOF {
			return err, nil
		}
		if len(line) > 0 {
			line = bytes.TrimSuffix(line, []byte{c.lineEnd})
			if c.fields {
				c.cutFields(line, out)
			} else {
				c.cutBytes(line, out)
			}
		}
		if err == io.EOF {
			return nil, nil
		}
		if err := out.Err(); err != nil {
			return nil, err
		}
	}
}

func (c *cutConfig) cutBytes(line []byte, out *cli.Output) {
	printed := false
	for index, b := range line {
		in, start := c.selected(uint64(index) + 1)
		if !in {
			continue
		}
		if start && printed && c.hasOutputDelimiter {
			out.Put(c.outputDelimiter)
		}
		out.PutByte(b)
		printed = true
	}
	out.PutByte(c.lineEnd)
}

func (c *cutConfig) cutFields(line []byte, out *cli.Output) {
	if bytes.IndexByte(line, c.delimiter) < 0 {
		if !c.onlyDelimited {
			out.Put(line)
			out.PutByte(c.lineEnd)
		}
		return
	}
	printed := false
	for field := uint64(1); ; field++ {
		end := bytes.IndexByte(line, c.delimiter)
		value := line
		if end >= 0 {
			value = line[:end]
		}
		if in, _ := c.selected(field); in {
			if printed {
				out.Put(c.outputDelimiter)
			}
			out.Put(value)
			printed = true
		}
		if end < 0 {
			break
		}
		line = line[end+1:]
	}
	out.PutByte(c.lineEnd)
}
