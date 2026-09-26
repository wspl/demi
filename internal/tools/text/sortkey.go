package text

import (
	"bytes"
	"crypto/md5"
	"fmt"
	"math/big"
	"strings"

	"github.com/wspl/demi/internal/tools/text/cli"
)

// sortKind is how a sort key compares.
type sortKind int

const (
	sortText sortKind = iota
	sortNumeric
	sortGeneralNumeric
	sortHumanNumeric
	sortMonth
	sortVersion
	sortRandom
)

// sortIgnore says which bytes a text comparison skips.
type sortIgnore int

const (
	ignoreNone sortIgnore = iota
	// ignoreNondictionary keeps only blanks and alphanumerics (-d).
	ignoreNondictionary
	// ignoreNonprinting keeps only printable bytes (-i).
	ignoreNonprinting
)

// sortOrdering are the ordering options of a key or of the whole command.
type sortOrdering struct {
	skipStartBlanks bool
	skipEndBlanks   bool
	ignore          sortIgnore
	fold            bool
	kind            sortKind
	reverse         bool
}

// set reports whether any ordering option is given.
func (o sortOrdering) set() bool {
	return o != sortOrdering{}
}

// applyFlag applies one ordering letter; it reports false for a letter
// that is not an ordering option. end says the letter follows POS2.
func (o *sortOrdering) applyFlag(letter byte, end bool) bool {
	switch letter {
	case 'b':
		if end {
			o.skipEndBlanks = true
		} else {
			o.skipStartBlanks = true
		}
	case 'd':
		o.ignore = ignoreNondictionary
	case 'f':
		o.fold = true
	case 'g':
		o.kind = sortGeneralNumeric
	case 'h':
		o.kind = sortHumanNumeric
	case 'i':
		o.ignore = ignoreNonprinting
	case 'M':
		o.kind = sortMonth
	case 'n':
		o.kind = sortNumeric
	case 'R':
		o.kind = sortRandom
	case 'r':
		o.reverse = true
	case 'V':
		o.kind = sortVersion
	default:
		return false
	}
	return true
}

// sortOrderingKinds are the kinds as flags, for the incompatibility check;
// the kinds are exclusive in sortOrdering, so the check tracks them apart.
type sortOrderingKinds struct {
	numeric, general, human, month, version, random bool
}

func (k *sortOrderingKinds) add(letter byte) {
	switch letter {
	case 'n':
		k.numeric = true
	case 'g':
		k.general = true
	case 'h':
		k.human = true
	case 'M':
		k.month = true
	case 'V':
		k.version = true
	case 'R':
		k.random = true
	}
}

// incompatible returns the conflicting options of a key as GNU sort names
// them ("-gn"), or "" when they are compatible.
func incompatible(o sortOrdering, kinds sortOrderingKinds) string {
	count := 0
	for _, kind := range []bool{kinds.numeric, kinds.general, kinds.human, kinds.month} {
		if kind {
			count++
		}
	}
	if kinds.version || kinds.random || o.ignore != ignoreNone {
		count++
	}
	if count <= 1 {
		return ""
	}
	var letters strings.Builder
	for _, flag := range []struct {
		on     bool
		letter byte
	}{
		{o.ignore == ignoreNondictionary, 'd'},
		{o.fold, 'f'},
		{kinds.general, 'g'},
		{kinds.human, 'h'},
		{o.ignore == ignoreNonprinting, 'i'},
		{kinds.month, 'M'},
		{kinds.numeric, 'n'},
		{kinds.random, 'R'},
		{kinds.version, 'V'},
	} {
		if flag.on {
			letters.WriteByte(flag.letter)
		}
	}
	return letters.String()
}

// sortKey is one -k key, or the whole line.
type sortKey struct {
	// startField and startChar are zero-based.
	startField, startChar uint64
	// endField is zero-based; endChar is one-based, 0 meaning the end of
	// the field. hasEnd is false for a key that runs to the end of line.
	endField, endChar uint64
	hasEnd            bool
	ordering          sortOrdering
	kinds             sortOrderingKinds
}

// parseSortKey parses a -k KEYDEF.
func parseSortKey(spec string) (sortKey, error) {
	invalid := func(problem string) error {
		return fmt.Errorf("%s: invalid field specification %s", problem, cli.Quote(spec))
	}
	key := sortKey{}
	rest := spec
	field, rest, err := parseKeyNumber(rest, "invalid number at field start")
	if err != nil {
		return key, err
	}
	if field == 0 {
		return key, invalid("field number is zero")
	}
	key.startField = field - 1
	if strings.HasPrefix(rest, ".") {
		var char uint64
		char, rest, err = parseKeyNumber(rest[1:], "invalid number after '.'")
		if err != nil {
			return key, err
		}
		if char == 0 {
			return key, invalid("character offset is zero")
		}
		key.startChar = char - 1
	}
	rest = key.flags(rest, false)
	if strings.HasPrefix(rest, ",") {
		key.hasEnd = true
		field, rest, err = parseKeyNumber(rest[1:], "invalid number after ','")
		if err != nil {
			return key, err
		}
		if field == 0 {
			return key, invalid("field number is zero")
		}
		key.endField = field - 1
		if strings.HasPrefix(rest, ".") {
			key.endChar, rest, err = parseKeyNumber(rest[1:], "invalid number after '.'")
			if err != nil {
				return key, err
			}
		}
		rest = key.flags(rest, true)
	}
	if rest != "" {
		return key, invalid("stray character in field spec")
	}
	return key, nil
}

// flags applies the ordering letters at the start of rest and returns the
// remainder.
func (k *sortKey) flags(rest string, end bool) string {
	for rest != "" && k.ordering.applyFlag(rest[0], end) {
		k.kinds.add(rest[0])
		rest = rest[1:]
	}
	return rest
}

// parseKeyNumber parses the decimal number at the start of text,
// saturating on overflow as GNU sort does.
func parseKeyNumber(text, problem string) (uint64, string, error) {
	body := strings.TrimPrefix(text, "+")
	digits := 0
	for digits < len(body) && body[digits] >= '0' && body[digits] <= '9' {
		digits++
	}
	if digits == 0 {
		return 0, text, fmt.Errorf("%s: invalid count at start of %s", problem, cli.Quote(text))
	}
	return parseSize(body[:digits]), body[digits:], nil
}

// sortBlank is a blank byte of the C locale for field splitting.
func sortBlank(c byte) bool {
	return c == ' ' || c == '\t'
}

// sortConfig is what compares two lines.
type sortConfig struct {
	keys []sortKey
	// global holds the options given outside -k; its reverse also reverses
	// the last-resort comparison.
	global sortOrdering
	// tab is the field separator; hasTab is false for blank-separated
	// fields.
	tab    byte
	hasTab bool
	// stable disables the last-resort comparison; unique implies it.
	stable bool
	unique bool
	salt   []byte
}

// keyText returns the part of line that key selects.
func (c *sortConfig) keyText(line []byte, key *sortKey) []byte {
	start := c.fieldStart(line, key.startField)
	if key.ordering.skipStartBlanks {
		for start < len(line) && sortBlank(line[start]) {
			start++
		}
	}
	start = int(min(uint64(len(line)), uint64(start)+key.startChar))
	end := len(line)
	if key.hasEnd {
		end = c.keyEnd(line, key)
	}
	if end < start {
		return line[start:start]
	}
	return line[start:end]
}

// fieldStart returns where zero-based field index starts.
func (c *sortConfig) fieldStart(line []byte, index uint64) int {
	position := 0
	for ; index > 0 && position < len(line); index-- {
		if c.hasTab {
			next := bytes.IndexByte(line[position:], c.tab)
			if next < 0 {
				return len(line)
			}
			position += next + 1
			continue
		}
		for position < len(line) && sortBlank(line[position]) {
			position++
		}
		for position < len(line) && !sortBlank(line[position]) {
			position++
		}
	}
	return position
}

// keyEnd returns where a key with an end position ends.
func (c *sortConfig) keyEnd(line []byte, key *sortKey) int {
	position := c.fieldStart(line, key.endField)
	if key.endChar == 0 {
		// The end of the field.
		if c.hasTab {
			next := bytes.IndexByte(line[position:], c.tab)
			if next < 0 {
				return len(line)
			}
			return position + next
		}
		for position < len(line) && sortBlank(line[position]) {
			position++
		}
		for position < len(line) && !sortBlank(line[position]) {
			position++
		}
		return position
	}
	if key.ordering.skipEndBlanks {
		for position < len(line) && sortBlank(line[position]) {
			position++
		}
	}
	return int(min(uint64(len(line)), uint64(position)+key.endChar))
}

// compare orders two lines: key by key, then by the whole line unless the
// sort is stable or unique.
func (c *sortConfig) compare(a, b []byte) int {
	if diff := c.compareKeys(a, b); diff != 0 {
		return diff
	}
	if c.stable || c.unique {
		return 0
	}
	diff := bytes.Compare(a, b)
	if c.global.reverse {
		return -diff
	}
	return diff
}

// compareKeys orders two lines by their keys only.
func (c *sortConfig) compareKeys(a, b []byte) int {
	for index := range c.keys {
		key := &c.keys[index]
		diff := c.compareKey(c.keyText(a, key), c.keyText(b, key), &key.ordering)
		if key.ordering.reverse {
			diff = -diff
		}
		if diff != 0 {
			return diff
		}
	}
	return 0
}

func (c *sortConfig) compareKey(a, b []byte, ordering *sortOrdering) int {
	switch ordering.kind {
	case sortNumeric:
		return compareNumbers(a, b)
	case sortGeneralNumeric:
		return compareGeneral(a, b)
	case sortHumanNumeric:
		return compareHuman(a, b)
	case sortMonth:
		return month(a) - month(b)
	case sortVersion:
		return compareVersions(string(a), string(b))
	case sortRandom:
		return c.compareRandom(a, b, ordering)
	case sortText:
	}
	return compareText(a, b, ordering)
}

// compareText compares with the ignore and fold options.
func compareText(a, b []byte, ordering *sortOrdering) int {
	if ordering.ignore == ignoreNone && !ordering.fold {
		return bytes.Compare(a, b)
	}
	i, j := 0, 0
	for {
		for i < len(a) && ignored(a[i], ordering.ignore) {
			i++
		}
		for j < len(b) && ignored(b[j], ordering.ignore) {
			j++
		}
		if i == len(a) || j == len(b) {
			return compareInts(len(a)-i, len(b)-j)
		}
		x := a[i]
		y := b[j]
		if ordering.fold {
			x = upper(x)
			y = upper(y)
		}
		if x != y {
			return compareInts(int(x), int(y))
		}
		i++
		j++
	}
}

// compareInts returns the sign of x - y; with remaining lengths it orders
// the input that ran out first.
func compareInts(x, y int) int {
	switch {
	case x < y:
		return -1
	case x > y:
		return 1
	}
	return 0
}

func ignored(c byte, ignore sortIgnore) bool {
	switch ignore {
	case ignoreNondictionary:
		return !sortBlank(c) && !isAlpha(c) && !isDigit(c)
	case ignoreNonprinting:
		return c < 32 || c >= 127
	case ignoreNone:
	}
	return false
}

func upper(c byte) byte {
	if c >= 'a' && c <= 'z' {
		return c - 'a' + 'A'
	}
	return c
}

// sortNumber is a decimal number as sort -n reads it: blanks, an optional
// minus sign, digits and a fraction. Anything else ends it; no digits is 0.
type sortNumber struct {
	negative bool
	// integer has no leading zeros; fraction has no trailing zeros.
	integer, fraction string
	// rest is what follows the number.
	rest []byte
}

func parseSortNumber(text []byte) sortNumber {
	position := 0
	for position < len(text) && sortBlank(text[position]) {
		position++
	}
	number := sortNumber{}
	if position < len(text) && text[position] == '-' {
		number.negative = true
		position++
	}
	start := position
	for position < len(text) && isDigit(text[position]) {
		position++
	}
	number.integer = strings.TrimLeft(string(text[start:position]), "0")
	if position < len(text) && text[position] == '.' {
		position++
		start = position
		for position < len(text) && isDigit(text[position]) {
			position++
		}
		number.fraction = strings.TrimRight(string(text[start:position]), "0")
	}
	number.rest = text[position:]
	if number.integer == "" && number.fraction == "" {
		number.negative = false
	}
	return number
}

// compareMagnitudes compares the absolute values of two numbers.
func (n sortNumber) compareMagnitude(other sortNumber) int {
	if diff := compareInts(len(n.integer), len(other.integer)); diff != 0 {
		return diff
	}
	if diff := strings.Compare(n.integer, other.integer); diff != 0 {
		return diff
	}
	return strings.Compare(n.fraction, other.fraction)
}

// compare orders two numbers by value.
func (n sortNumber) compare(other sortNumber) int {
	if n.negative != other.negative {
		if n.negative {
			return -1
		}
		return 1
	}
	diff := n.compareMagnitude(other)
	if n.negative {
		return -diff
	}
	return diff
}

func compareNumbers(a, b []byte) int {
	return parseSortNumber(a).compare(parseSortNumber(b))
}

// humanUnits orders the suffixes of sort -h.
const humanUnits = "KMGTPEZYRQ"

// humanOrder is the signed rank of a number's unit suffix; a number without
// a nonzero digit ranks 0 whatever its suffix.
func humanOrder(number sortNumber) int {
	if number.integer == "" && number.fraction == "" {
		return 0
	}
	order := 0
	if len(number.rest) > 0 {
		suffix := number.rest[0]
		if suffix == 'k' {
			suffix = 'K'
		}
		order = strings.IndexByte(humanUnits, suffix) + 1
	}
	if number.negative {
		return -order
	}
	return order
}

func compareHuman(a, b []byte) int {
	x := parseSortNumber(a)
	y := parseSortNumber(b)
	if diff := compareInts(humanOrder(x), humanOrder(y)); diff != 0 {
		return diff
	}
	return x.compare(y)
}

// compareGeneral compares as sort -g: numbers by value after anything that
// is not a number, and NaNs between them.
func compareGeneral(a, b []byte) int {
	x := parseGeneral(a)
	y := parseGeneral(b)
	switch {
	case x.kind != y.kind:
		return compareInts(int(x.kind), int(y.kind))
	case x.kind == generalNumber:
		return x.value.Cmp(y.value)
	}
	return 0
}

// generalKind orders the values sort -g sees.
type generalKind int

const (
	generalInvalid generalKind = iota
	generalNaN
	generalNumber
)

// generalValue is a key as sort -g reads it.
type generalValue struct {
	kind  generalKind
	value *big.Float
}

// generalPrecision is the mantissa of the C long double that GNU sort -g
// compares with; big.Float also has its exponent range.
const generalPrecision = 64

// parseGeneral parses the longest prefix of text that is a C floating
// number after leading white space, as strtold does.
func parseGeneral(text []byte) generalValue {
	s := strings.TrimLeft(string(text), " \t\n\v\f\r")
	position := 0
	if position < len(s) && (s[position] == '+' || s[position] == '-') {
		position++
	}
	sign := s[:position]
	body := strings.ToLower(s[position:])
	number := ""
	switch {
	case strings.HasPrefix(body, "inf"):
		value := new(big.Float).SetInf(sign == "-")
		return generalValue{kind: generalNumber, value: value}
	case strings.HasPrefix(body, "nan"):
		return generalValue{kind: generalNaN}
	case strings.HasPrefix(body, "0x") && scanDigits(body[2:], isHexDigit) > 0:
		number = hexFloat(body)
	default:
		number = body[:scanDecimalFloat(body)]
	}
	if number == "" {
		return generalValue{kind: generalInvalid}
	}
	value, _, err := big.ParseFloat(sign+number, 0, generalPrecision, big.ToNearestEven)
	if err != nil {
		return generalValue{kind: generalInvalid}
	}
	return generalValue{kind: generalNumber, value: value}
}

// scanDecimalFloat returns the length of the decimal number at the start of
// s: digits, a fraction and an exponent; 0 when there are no digits.
func scanDecimalFloat(s string) int {
	integer := scanDigits(s, isDigit)
	position := integer
	fraction := 0
	if position < len(s) && s[position] == '.' {
		fraction = scanDigits(s[position+1:], isDigit)
		if integer+fraction > 0 {
			position += 1 + fraction
		}
	}
	if integer+fraction == 0 {
		return 0
	}
	if position < len(s) && s[position] == 'e' {
		exponent := position + 1
		if exponent < len(s) && (s[exponent] == '+' || s[exponent] == '-') {
			exponent++
		}
		if digits := scanDigits(s[exponent:], isDigit); digits > 0 {
			position = exponent + digits
		}
	}
	return position
}

// hexFloat returns the hexadecimal float at the start of body (after
// "0x"), with a binary exponent, which strtold may omit and big.ParseFloat
// requires.
func hexFloat(body string) string {
	position := 2 + scanDigits(body[2:], isHexDigit)
	if position < len(body) && body[position] == '.' {
		position += 1 + scanDigits(body[position+1:], isHexDigit)
	}
	if position < len(body) && body[position] == 'p' {
		start := position + 1
		if start < len(body) && (body[start] == '+' || body[start] == '-') {
			start++
		}
		if digits := scanDigits(body[start:], isDigit); digits > 0 {
			return body[:start+digits]
		}
	}
	return body[:position] + "p0"
}

func scanDigits(s string, accept func(byte) bool) int {
	length := 0
	for length < len(s) && accept(s[length]) {
		length++
	}
	return length
}

func isHexDigit(c byte) bool {
	return isDigit(c) || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F')
}

// months are the month abbreviations of the C locale.
var months = []string{"JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"}

// month returns 1 to 12 for a key that starts with a month name after
// blanks, and 0 otherwise.
func month(text []byte) int {
	text = bytes.TrimLeft(text, " \t\n\v\f\r")
	if len(text) < 3 {
		return 0
	}
	name := strings.ToUpper(string(text[:3]))
	for index, abbreviation := range months {
		if name == abbreviation {
			return index + 1
		}
	}
	return 0
}

// compareRandom orders keys by a salted hash, keeping equal keys together.
func (c *sortConfig) compareRandom(a, b []byte, ordering *sortOrdering) int {
	hashA := md5.Sum(append(append([]byte{}, c.salt...), a...))
	hashB := md5.Sum(append(append([]byte{}, c.salt...), b...))
	if diff := bytes.Compare(hashA[:], hashB[:]); diff != 0 {
		return diff
	}
	return compareText(a, b, ordering)
}
