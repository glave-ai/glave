package csv

import (
	"reflect"
	"testing"
)

func TestPlainFields(t *testing.T) {
	// PASS_TO_PASS — unquoted fields split normally.
	got := ParseLine("a,b,c")
	if !reflect.DeepEqual(got, []string{"a", "b", "c"}) {
		t.Fatalf("got %#v", got)
	}
}

func TestQuotedComma(t *testing.T) {
	// FAIL_TO_PASS — a comma inside a quoted field is part of the value, and the
	// quotes are stripped.
	got := ParseLine(`a,"b,c",d`)
	if !reflect.DeepEqual(got, []string{"a", "b,c", "d"}) {
		t.Fatalf("got %#v", got)
	}
}
