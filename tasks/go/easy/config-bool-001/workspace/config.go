package config

import "strconv"

// Settings holds service configuration parsed from environment values.
type Settings struct {
	Port    int
	Debug   bool
	Timeout int
}

// Parse builds Settings from environment values, applying defaults for any
// missing keys.
func Parse(env map[string]string) Settings {
	s := Settings{Port: 8080, Debug: false, Timeout: 30}
	if v, ok := env["PORT"]; ok {
		s.Port = atoi(v, s.Port)
	}
	// BUG: treats the mere presence of DEBUG as enabling it, so DEBUG="false"
	// wrongly turns debug on. Should parse the value.
	if _, ok := env["DEBUG"]; ok {
		s.Debug = true
	}
	if v, ok := env["TIMEOUT"]; ok {
		s.Timeout = atoi(v, s.Timeout)
	}
	return s
}

func atoi(s string, def int) int {
	if n, err := strconv.Atoi(s); err == nil {
		return n
	}
	return def
}
