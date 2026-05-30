package csv

import "strings"

// ParseLine splits a single CSV line into fields. Double-quoted fields may
// contain commas, which must not be treated as field separators; the surrounding
// quotes are not part of the field value.
func ParseLine(line string) []string {
	// BUG: naive split treats every comma as a separator, so it breaks quoted
	// fields that contain commas (and leaves the quote characters in place).
	return strings.Split(line, ",")
}
