package config

import "testing"

func TestDefaults(t *testing.T) {
	// PASS_TO_PASS — empty env yields documented defaults.
	s := Parse(map[string]string{})
	if s.Port != 8080 || s.Debug != false || s.Timeout != 30 {
		t.Fatalf("unexpected defaults: %+v", s)
	}
}

func TestPortAndTimeout(t *testing.T) {
	// PASS_TO_PASS — numeric overrides parse correctly.
	s := Parse(map[string]string{"PORT": "9090", "TIMEOUT": "5"})
	if s.Port != 9090 || s.Timeout != 5 {
		t.Fatalf("got %+v", s)
	}
}

func TestDebugFalseDisablesDebug(t *testing.T) {
	// FAIL_TO_PASS — DEBUG="false" must NOT enable debug.
	s := Parse(map[string]string{"DEBUG": "false"})
	if s.Debug {
		t.Fatalf(`DEBUG="false" should disable debug, got Debug=true`)
	}
}
