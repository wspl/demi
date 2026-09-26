package text

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"math"
	"regexp"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/tools/text/cli"
)

const tailUsage = `Usage: %[1]s [OPTION]... [FILE]...
Print the last 10 lines of each FILE to standard output.
With more than one FILE, precede each with a header giving the file name.

With no FILE, or when FILE is -, read standard input.

  -c, --bytes=[+]NUM       output the last NUM bytes; or use -c +NUM to
                             output starting with byte NUM of each file
  -f, --follow[={name|descriptor}]
                           output appended data as the file grows;
                             an absent option argument means 'descriptor'
  -F                       same as --follow=name --retry
  -n, --lines=[+]NUM       output the last NUM lines, instead of the last 10;
                             or use -n +NUM to skip NUM-1 lines at the start
      --max-unchanged-stats=N
                           accepted for compatibility; every check reopens
                             a file followed by name
      --pid=PID            with -f, terminate after process ID, PID dies
  -q, --quiet, --silent    never output headers giving file names
      --retry              keep trying to open a file if it is inaccessible
  -s, --sleep-interval=N   with -f, sleep for approximately N seconds
                             (default 1.0) between checks
  -v, --verbose            always output headers giving file names
  -z, --zero-terminated    line delimiter is NUL, not newline
      --help               display this help and exit
      --version            output version information and exit

NUM may have a multiplier suffix: b 512, kB 1000, K 1024, MB 1000*1000,
M 1024*1024, GB 1000*1000*1000, G 1024*1024*1024, and so on for T, P, E, Z,
Y, R, Q. Binary prefixes can be used, too: KiB=K, MiB=M, and so on.
`

var tailOptions = []cli.Option{
	{Short: 'c', Long: "bytes", Arg: cli.RequiredArg},
	{Short: 'f', Long: "follow", Arg: cli.OptionalArg},
	{Short: 'F', Key: "F"},
	{Short: 'n', Long: "lines", Arg: cli.RequiredArg},
	{Long: "max-unchanged-stats", Arg: cli.RequiredArg},
	{Long: "pid", Arg: cli.RequiredArg},
	{Short: 'q', Long: "quiet"},
	{Long: "silent", Key: "quiet"},
	{Long: "retry"},
	{Short: 's', Long: "sleep-interval", Arg: cli.RequiredArg},
	{Short: 'v', Long: "verbose"},
	{Short: 'z', Long: "zero-terminated"},
}

func init() {
	// Digits are options only to reject a misplaced obsolete -NUM.
	for digit := '0'; digit <= '9'; digit++ {
		tailOptions = append(tailOptions, cli.Option{Short: digit, Key: "digit"})
	}
}

// followMode is what tail -f follows.
type followMode int

const (
	followNone followMode = iota
	followDescriptor
	followName
)

// obsoleteTail matches the obsolete first argument [+-]NUM[bcl][f].
var obsoleteTail = regexp.MustCompile(`^(?:\+[0-9]*|-[0-9]+)[bcl]?f?$`)

// tailSpec is the part tail prints: the last count lines or bytes, or
// everything from line or byte count on.
type tailSpec struct {
	bytes     bool
	count     uint64
	fromStart bool
}

// tailConfig is a parsed tail command line.
type tailConfig struct {
	spec      tailSpec
	follow    followMode
	retry     bool
	pid       int
	hasPID    bool
	interval  time.Duration
	show      headers
	delimiter byte
}

func tail(inv *toolctx.Invocation, args []string) int {
	prog := &cli.Program{Name: args[0], Inv: inv, Usage: tailUsage, BadUsage: 1}
	config := tailConfig{spec: tailSpec{count: 10}, interval: time.Second, delimiter: '\n'}
	rest := args[1:]
	if isObsoleteTail(rest) {
		obsolete := rest[0]
		rest = rest[1:]
		if err := config.parseObsolete(obsolete); err != nil {
			prog.Errorf("%s", err)
			return 1
		}
	}
	options, operands, status, done := prog.Parse(rest, tailOptions)
	if done {
		return status
	}
	for _, option := range options {
		if err := config.apply(option); err != nil {
			var usage *cli.UsageError
			if errors.As(err, &usage) {
				return prog.UsageError(err)
			}
			prog.Errorf("%s", err)
			return 1
		}
	}
	if config.hasPID && config.follow == followNone {
		prog.Errorf("warning: PID ignored; --pid=PID is useful only when following")
	}
	if config.retry {
		switch config.follow {
		case followNone:
			prog.Errorf("warning: --retry ignored; --retry is useful only when following")
		case followDescriptor:
			prog.Errorf("warning: --retry only effective for the initial open")
		case followName:
		}
	}
	if len(operands) == 0 {
		operands = []string{cli.StdinName}
	}
	run := &tailRun{
		prog:   prog,
		inv:    inv,
		config: config,
		stdout: cli.Writer(inv.Context, inv.Stdout),
		first:  true,
		headers: config.show == headersAlways ||
			(config.show == headersAuto && len(operands) > 1),
	}
	return run.run(operands)
}

// isObsoleteTail reports whether the first argument is the obsolete
// [+-]NUM form, which tail accepts only before at most one file operand.
func isObsoleteTail(args []string) bool {
	switch {
	case len(args) == 0 || !obsoleteTail.MatchString(args[0]):
		return false
	case len(args) == 1 || len(args) == 2:
		return true
	default:
		return len(args) == 3 && args[1] == "--"
	}
}

// parseObsolete applies the obsolete [+-]NUM[bcl][f] argument.
func (c *tailConfig) parseObsolete(arg string) error {
	body := arg[1:]
	c.spec = tailSpec{fromStart: arg[0] == '+'}
	multiplier := uint64(1)
	if strings.HasSuffix(body, "f") {
		c.follow = followDescriptor
		body = strings.TrimSuffix(body, "f")
	}
	if body != "" {
		switch body[len(body)-1] {
		case 'b':
			c.spec.bytes = true
			multiplier = 512
			body = body[:len(body)-1]
		case 'c':
			c.spec.bytes = true
			body = body[:len(body)-1]
		case 'l':
			body = body[:len(body)-1]
		}
	}
	count := uint64(10)
	if body != "" {
		var err error
		count, err = cli.ParseUint(body, "")
		if err == nil && count > 0 && multiplier > math.MaxUint64/count {
			err = cli.ErrOverflow
		}
		if err != nil {
			return errors.New(countError(c.spec.bytes, body, err))
		}
	}
	c.spec.count = count * multiplier
	return nil
}

// apply applies one option.
func (c *tailConfig) apply(option cli.Parsed) error {
	switch option.Key {
	case "bytes", "lines":
		value := option.Value
		c.spec = tailSpec{bytes: option.Key == "bytes"}
		switch {
		case strings.HasPrefix(value, "+"):
			c.spec.fromStart = true
			value = value[1:]
		case strings.HasPrefix(value, "-"):
			value = value[1:]
		}
		count, err := cli.ParseUint(value, cli.SizeSuffixes)
		if err != nil {
			return errors.New(countError(c.spec.bytes, option.Value, err))
		}
		c.spec.count = count
	case "follow":
		c.follow = followDescriptor
		if option.HasValue {
			mode, err := cli.Argmatch("--follow", option.Value, []cli.Choice{
				{Key: "descriptor", Words: []string{"descriptor"}},
				{Key: "name", Words: []string{"name"}},
			})
			if err != nil {
				return err
			}
			if mode == "name" {
				c.follow = followName
			}
		}
	case "F":
		c.follow = followName
		c.retry = true
	case "max-unchanged-stats":
		if _, err := cli.ParseUint(option.Value, ""); err != nil {
			return fmt.Errorf("invalid maximum number of unchanged stats between opens: %s", cli.Quote(option.Value))
		}
	case "pid":
		pid, err := strconv.ParseInt(strings.TrimLeft(option.Value, " \t\n\v\f\r"), 10, 32)
		if err != nil || pid < 0 {
			return fmt.Errorf("invalid PID: %s", cli.Quote(option.Value))
		}
		c.pid = int(pid)
		c.hasPID = true
	case "quiet":
		c.show = headersNever
	case "verbose":
		c.show = headersAlways
	case "retry":
		c.retry = true
	case "sleep-interval":
		seconds, err := strconv.ParseFloat(strings.TrimLeft(option.Value, " \t\n\v\f\r"), 64)
		if err != nil || seconds < 0 || math.IsNaN(seconds) {
			return fmt.Errorf("invalid number of seconds: %s", cli.Quote(option.Value))
		}
		c.interval = time.Duration(math.Min(seconds*float64(time.Second), math.MaxInt64))
	case "zero-terminated":
		c.delimiter = 0
	case "digit":
		return fmt.Errorf("option used in invalid context -- %s", strings.TrimPrefix(option.Name, "-"))
	}
	return nil
}

// tailRun is one run of tail over its operands.
type tailRun struct {
	prog    *cli.Program
	inv     *toolctx.Invocation
	config  tailConfig
	stdout  io.Writer
	headers bool
	// first is true until the first header is printed.
	first  bool
	status int
}

// tailFile is one operand, followed or not.
type tailFile struct {
	name string
	// file is open while the operand is followed through it.
	file toolctx.File
	// state says whether the operand is still followed.
	state tailFileState
}

// tailFileState is where a followed operand stands.
type tailFileState int

const (
	// tailOpen: the file is open and followed.
	tailOpen tailFileState = iota
	// tailWaiting: the name is followed but cannot be opened now.
	tailWaiting
	// tailDropped: the operand is no longer followed.
	tailDropped
	// tailIgnored: the operand is standard input, which -f ignores.
	tailIgnored
)

func (r *tailRun) run(operands []string) int {
	files := make([]*tailFile, len(operands))
	for index, name := range operands {
		file, err := r.initial(name)
		if err != nil {
			return r.prog.WriteError(err)
		}
		files[index] = file
	}
	ignored := 0
	for _, file := range files {
		if file.state == tailIgnored {
			ignored++
		}
	}
	if r.config.follow == followNone || ignored == len(files) {
		for _, file := range files {
			r.closeFile(file)
		}
		return r.status
	}
	return r.followLoop(files)
}

// initial prints the selected part of one operand and returns how it is
// followed. The error is a failed write to standard output.
func (r *tailRun) initial(name string) (*tailFile, error) {
	entry := &tailFile{name: name, state: tailDropped}
	input, err := cli.Open(r.inv, name)
	if err != nil {
		r.prog.Errorf("cannot open %s for reading: %s", cli.Quote(name), cli.Strerror(err))
		r.status = 1
		if r.config.follow == followName && r.config.retry {
			entry.state = tailWaiting
		}
		return entry, nil
	}
	if err := r.header(name); err != nil {
		input.Release()
		return nil, err
	}
	readErr, writeErr := r.printTail(input)
	if writeErr != nil {
		input.Release()
		return nil, writeErr
	}
	if readErr != nil {
		r.prog.Errorf("error reading %s: %s", cli.Quote(name), cli.Strerror(readErr))
		r.status = 1
		if r.config.follow != followNone {
			r.prog.Errorf("%s: cannot follow end of this type of file; giving up on this name", cli.QuoteName(name))
		}
		input.Release()
		return entry, nil
	}
	if input.File == nil {
		// Standard input is a stream, not a file tail can come back to;
		// following ignores it, as GNU tail ignores a pipe.
		entry.state = tailIgnored
		return entry, nil
	}
	entry.file = input.File
	entry.state = tailOpen
	return entry, nil
}

// header prints the header for an operand when headers are on.
func (r *tailRun) header(name string) error {
	if !r.headers {
		return nil
	}
	_, err := fmt.Fprintf(r.stdout, "%s==> %s <==\n", separator(r.first), displayName(name))
	r.first = false
	return err
}

func (r *tailRun) closeFile(file *tailFile) {
	if file.file != nil {
		// The file was only read; a close error loses nothing.
		_ = file.file.Close()
		file.file = nil
	}
}

// printTail prints the part of one input that the count selects, leaving a
// file positioned at its end.
func (r *tailRun) printTail(input *cli.Input) (readErr, writeErr error) {
	spec := r.config.spec
	if spec.fromStart {
		skip := spec.count
		if skip > 0 {
			skip--
		}
		if spec.bytes {
			return skipBytes(r.stdout, input, skip)
		}
		return skipLines(r.stdout, input, skip, r.config.delimiter)
	}
	if input.File != nil {
		if info, err := input.File.Stat(); err == nil && info.Mode().IsRegular() {
			return r.printFileEnd(input, info.Size())
		}
	}
	if spec.bytes {
		return lastBytes(r.stdout, input, spec.count)
	}
	return lastLines(r.stdout, input, spec.count, r.config.delimiter)
}

// printFileEnd prints the end of a regular file of the given size by
// reading it backwards from its end.
func (r *tailRun) printFileEnd(input *cli.Input, size int64) (readErr, writeErr error) {
	start := int64(0)
	count := r.config.spec.count
	if r.config.spec.bytes {
		if count < uint64(size) {
			start = size - int64(count)
		}
	} else {
		var err error
		start, err = lineStart(input.File, size, count, r.config.delimiter)
		if err != nil {
			return err, nil
		}
	}
	if _, err := input.File.Seek(start, io.SeekStart); err != nil {
		return err, nil
	}
	return copyAll(r.stdout, input)
}

// lineStart finds where the last count lines of a file of the given size
// start. A final line without a delimiter counts as a line.
func lineStart(file io.ReaderAt, size int64, count uint64, delimiter byte) (int64, error) {
	if count == 0 {
		return size, nil
	}
	buffer := make([]byte, 64*1024)
	end := size
	// The delimiter that ends the file ends the last line; it starts none.
	skipFinal := true
	for end > 0 {
		begin := end - int64(len(buffer))
		if begin < 0 {
			begin = 0
		}
		chunk := buffer[:end-begin]
		if _, err := file.ReadAt(chunk, begin); err != nil && err != io.EOF {
			return 0, err
		}
		for index := len(chunk) - 1; index >= 0; index-- {
			if chunk[index] != delimiter {
				skipFinal = false
				continue
			}
			if skipFinal {
				skipFinal = false
				continue
			}
			count--
			if count == 0 {
				return begin + int64(index) + 1, nil
			}
		}
		end = begin
	}
	return 0, nil
}

// copyAll copies the rest of an input.
func copyAll(stdout io.Writer, input io.Reader) (readErr, writeErr error) {
	buffer := make([]byte, 64*1024)
	for {
		read, err := input.Read(buffer)
		if read > 0 {
			if _, err := stdout.Write(buffer[:read]); err != nil {
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
}

// skipBytes drops the first n bytes and copies the rest.
func skipBytes(stdout io.Writer, input io.Reader, n uint64) (readErr, writeErr error) {
	buffer := make([]byte, 64*1024)
	for n > 0 {
		size := uint64(len(buffer))
		if n < size {
			size = n
		}
		read, err := input.Read(buffer[:size])
		n -= uint64(read)
		if err == io.EOF {
			return nil, nil
		}
		if err != nil {
			return err, nil
		}
	}
	return copyAll(stdout, input)
}

// skipLines drops the first n lines and copies the rest.
func skipLines(stdout io.Writer, input io.Reader, n uint64, delimiter byte) (readErr, writeErr error) {
	buffer := make([]byte, 64*1024)
	for n > 0 {
		read, err := input.Read(buffer)
		chunk := buffer[:read]
		for n > 0 {
			found := bytes.IndexByte(chunk, delimiter)
			if found < 0 {
				chunk = nil
				break
			}
			chunk = chunk[found+1:]
			n--
		}
		if len(chunk) > 0 {
			if _, err := stdout.Write(chunk); err != nil {
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
	return copyAll(stdout, input)
}

// lastBytes prints the last n bytes of a stream.
func lastBytes(stdout io.Writer, input io.Reader, n uint64) (readErr, writeErr error) {
	var held []byte
	buffer := make([]byte, 64*1024)
	for {
		read, err := input.Read(buffer)
		held = append(held, buffer[:read]...)
		if excess := uint64(len(held)); excess > n {
			held = append(held[:0], held[excess-n:]...)
		}
		if err == io.EOF {
			break
		}
		if err != nil {
			return err, nil
		}
	}
	_, err := stdout.Write(held)
	return nil, err
}

// lastLines prints the last n lines of a stream, holding only those lines.
func lastLines(stdout io.Writer, input io.Reader, n uint64, delimiter byte) (readErr, writeErr error) {
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
		lines := uint64(len(ends))
		if len(ends) == 0 || ends[len(ends)-1] != len(held) {
			if len(held) > 0 {
				lines++
			}
		}
		if lines > n {
			drop := ends[lines-n-1]
			held = append(held[:0], held[drop:]...)
			ends = ends[lines-n:]
			for index := range ends {
				ends[index] -= drop
			}
		}
		if err == io.EOF {
			break
		}
		if err != nil {
			return err, nil
		}
	}
	_, err := stdout.Write(held)
	return nil, err
}

// followLoop prints what is appended to the followed files until the job
// is cancelled, the --pid process ends, or no file is left to follow.
func (r *tailRun) followLoop(files []*tailFile) int {
	defer func() {
		for _, file := range files {
			r.closeFile(file)
		}
	}()
	// Like GNU tail, the first output while following names its file
	// unless it is the last operand.
	last := len(files) - 1
	ticker := time.NewTicker(max(r.config.interval, time.Millisecond))
	defer ticker.Stop()
	for {
		if !r.anyFollowed(files) {
			r.prog.Errorf("no files remaining")
			return 1
		}
		processEnded := r.config.hasPID && !processAlive(r.config.pid)
		for index, file := range files {
			printed, err := r.check(file, index != last)
			if err != nil {
				return r.prog.WriteError(err)
			}
			if printed {
				last = index
			}
		}
		if processEnded {
			return r.status
		}
		select {
		case <-r.inv.Context.Done():
			return 1
		case <-ticker.C:
		}
	}
}

func (r *tailRun) anyFollowed(files []*tailFile) bool {
	for _, file := range files {
		if file.state == tailOpen || file.state == tailWaiting {
			return true
		}
	}
	return false
}

// check looks at one followed operand once and prints what was appended,
// with a header first when needHeader is set. It reports whether it printed.
func (r *tailRun) check(file *tailFile, needHeader bool) (bool, error) {
	if r.config.follow == followName && (file.state == tailOpen || file.state == tailWaiting) {
		r.checkName(file)
	}
	if file.state != tailOpen {
		return false, nil
	}
	info, err := file.file.Stat()
	if err != nil {
		r.prog.Errorf("%s: %s", cli.QuoteName(file.name), cli.Strerror(err))
		r.status = 1
		r.closeFile(file)
		file.state = tailDropped
		return false, nil
	}
	position, err := file.file.Seek(0, io.SeekCurrent)
	if err != nil {
		return false, nil
	}
	if info.Mode().IsRegular() && info.Size() < position {
		r.prog.Errorf("%s: file truncated", cli.QuoteName(file.name))
		if _, err := file.file.Seek(0, io.SeekStart); err != nil {
			return false, nil
		}
		position = 0
	}
	if info.Size() == position {
		return false, nil
	}
	if needHeader {
		if err := r.header(file.name); err != nil {
			return false, err
		}
	}
	readErr, writeErr := copyAll(r.stdout, cli.Reader(r.inv.Context, file.file))
	if writeErr != nil {
		return true, writeErr
	}
	if readErr != nil {
		r.prog.Errorf("error reading %s: %s", cli.Quote(file.name), cli.Strerror(readErr))
		r.status = 1
	}
	return true, nil
}

// checkName follows an operand by name: it notices when the name stops
// naming a readable file, when a file appears under it, and when another
// file replaces it.
func (r *tailRun) checkName(file *tailFile) {
	named, err := r.inv.Files.Stat(file.name)
	if err != nil {
		if file.state == tailOpen {
			r.prog.Errorf("%s has become inaccessible: %s", cli.Quote(file.name), cli.Strerror(err))
			r.closeFile(file)
			file.state = tailWaiting
			if !r.config.retry {
				file.state = tailDropped
			}
		}
		return
	}
	if file.state == tailOpen {
		current, err := file.file.Stat()
		if err == nil && sameFile(current, named) {
			return
		}
		r.prog.Errorf("%s has been replaced;  following new file", cli.Quote(file.name))
		r.closeFile(file)
	} else {
		r.prog.Errorf("%s has appeared;  following new file", cli.Quote(file.name))
	}
	opened, err := r.inv.Files.Open(file.name)
	if err != nil {
		file.state = tailWaiting
		return
	}
	file.file = opened
	file.state = tailOpen
}

// sameFile reports whether two infos describe the same file. Files that do
// not expose their device and inode count as the same.
func sameFile(a, b fs.FileInfo) bool {
	statA, okA := a.Sys().(*syscall.Stat_t)
	statB, okB := b.Sys().(*syscall.Stat_t)
	if !okA || !okB {
		return true
	}
	return statA.Dev == statB.Dev && statA.Ino == statB.Ino
}

// processAlive reports whether process pid exists; signal 0 only checks.
func processAlive(pid int) bool {
	err := syscall.Kill(pid, 0)
	return err == nil || errors.Is(err, syscall.EPERM)
}
