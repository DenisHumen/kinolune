// SPDX-License-Identifier: MPL-2.0

package settings

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func at(h, m int) time.Time { return time.Date(2026, 11, 25, h, m, 0, 0, time.UTC) }

func TestParseWindow(t *testing.T) {
	good := map[string]Window{
		"03:00-05:00": {180, 300},
		"23:30-01:00": {1410, 60},
		"00:00-23:59": {0, 1439},
	}
	for in, want := range good {
		got, err := ParseWindow(in)
		if err != nil || got != want {
			t.Errorf("ParseWindow(%q) = %v, %v; want %v", in, got, err, want)
		}
	}
	for _, in := range []string{"", "3:00-5:00", "03:00-03:00", "24:00-01:00", "03:60-04:00", "03:00_05:00", "aa:bb-cc:dd", "03:00-05:00 "} {
		if _, err := ParseWindow(in); err == nil {
			t.Errorf("ParseWindow(%q): want error", in)
		}
	}
}

func TestWindowContains(t *testing.T) {
	w, _ := ParseWindow("03:00-05:00")
	for _, tc := range []struct {
		h, m int
		in   bool
	}{{2, 59, false}, {3, 0, true}, {4, 59, true}, {5, 0, false}, {15, 0, false}} {
		if got := w.Contains(at(tc.h, tc.m)); got != tc.in {
			t.Errorf("03-05 contains %02d:%02d = %v", tc.h, tc.m, got)
		}
	}
	wrap, _ := ParseWindow("23:30-01:00")
	for _, tc := range []struct {
		h, m int
		in   bool
	}{{23, 29, false}, {23, 30, true}, {0, 0, true}, {0, 59, true}, {1, 0, false}, {12, 0, false}} {
		if got := wrap.Contains(at(tc.h, tc.m)); got != tc.in {
			t.Errorf("23:30-01 contains %02d:%02d = %v", tc.h, tc.m, got)
		}
	}
}

func TestWindowNextStart(t *testing.T) {
	w, _ := ParseWindow("03:00-05:00")
	if got := w.NextStart(at(1, 0)); !got.Equal(at(3, 0)) {
		t.Errorf("from 01:00: %v", got)
	}
	if got := w.NextStart(at(4, 0)); !got.Equal(at(4, 0)) {
		t.Errorf("inside: %v", got)
	}
	if got := w.NextStart(at(6, 0)); !got.Equal(at(3, 0).AddDate(0, 0, 1)) {
		t.Errorf("after: %v", got)
	}
	wrap, _ := ParseWindow("23:30-01:00")
	if got := wrap.NextStart(at(12, 0)); !got.Equal(at(23, 30)) {
		t.Errorf("wrap from noon: %v", got)
	}
}

func TestValidate(t *testing.T) {
	if err := Default().Validate(); err != nil {
		t.Fatal(err)
	}
	bad := []Settings{
		{Channel: "nightly", Auto: AutoInstall, Window: "03:00-05:00"},
		{Channel: "stable", Auto: "always", Window: "03:00-05:00"},
		{Channel: "stable", Auto: AutoOff, Window: "soon"},
	}
	for _, s := range bad {
		if err := s.Validate(); err == nil {
			t.Errorf("%+v: want error", s)
		}
	}
}

func TestAutoModeJSON(t *testing.T) {
	for in, want := range map[string]AutoMode{`true`: AutoInstall, `false`: AutoOff, `"download"`: AutoDownload} {
		var m AutoMode
		if err := json.Unmarshal([]byte(in), &m); err != nil || m != want {
			t.Errorf("%s -> %q, %v", in, m, err)
		}
	}
	var m AutoMode
	if err := json.Unmarshal([]byte(`3`), &m); err == nil {
		t.Error("number must fail")
	}
}

func TestPatch(t *testing.T) {
	beta, w := "beta", "01:00-02:00"
	s := Default().Apply(Patch{Channel: &beta, Window: &w})
	if s.Channel != "beta" || s.Window != w || s.Auto != AutoInstall {
		t.Errorf("patched = %+v", s)
	}
}

func TestStore(t *testing.T) {
	dir := t.TempDir()
	st := Store{Path: filepath.Join(dir, "sub", "settings.json")}
	s, err := st.Load()
	if err != nil || s != Default() {
		t.Fatalf("missing file: %+v %v", s, err)
	}
	want := Settings{Channel: "beta", Auto: AutoDownload, Window: "23:30-01:00"}
	if err := st.Save(want); err != nil {
		t.Fatal(err)
	}
	got, err := (Store{Path: st.Path}).Load()
	if err != nil || got != want {
		t.Fatalf("reload: %+v %v", got, err)
	}
	fi, _ := os.Stat(st.Path)
	if fi.Mode().Perm() != 0o600 {
		t.Errorf("mode = %v", fi.Mode())
	}
	if err := st.Save(Settings{Channel: "x"}); err == nil {
		t.Error("invalid settings must not be saved")
	}
	entries, _ := os.ReadDir(filepath.Dir(st.Path))
	if len(entries) != 1 {
		t.Errorf("temp files left: %v", entries)
	}
	if err := os.WriteFile(st.Path, []byte("{broken"), 0o600); err != nil {
		t.Fatal(err)
	}
	if s, err := st.Load(); err == nil || s != Default() {
		t.Errorf("broken file: %+v %v", s, err)
	}
	if err := os.WriteFile(st.Path, []byte(`{"channel":"gamma"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := st.Load(); err == nil {
		t.Error("invalid stored channel must be reported")
	}
}
