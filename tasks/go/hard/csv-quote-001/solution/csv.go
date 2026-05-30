package csv

import "strings"

// ParseLine splits a single CSV line into fields, honoring double-quoted fields
// that may contain commas (the quotes are stripped from the value).
func ParseLine(line string) []string {
	var fields []string
	var cur strings.Builder
	inQuotes := false
	for _, r := range line {
		switch {
		case r == '"':
			inQuotes = !inQuotes
		case r == ',' && !inQuotes:
			fields = append(fields, cur.String())
			cur.Reset()
		default:
			cur.WriteRune(r)
		}
	}
	fields = append(fields, cur.String())
	return fields
}
