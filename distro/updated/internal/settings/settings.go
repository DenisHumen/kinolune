// SPDX-License-Identifier: MPL-2.0

// Package settings holds the user settings of alpha-updated (channel,
// automatic mode, maintenance window) and stores them as JSON under
// /var/lib/alpha/updated.
package settings

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

// Channels a device can follow. A channel is a tag in the image registry
// (research 05 §4.2 p.2).
var Channels = []string{"stable", "beta"}

// AutoMode says what the agent does on its own.
type AutoMode string

const (
	// AutoOff: only periodic checks, the user sees "update available".
	AutoOff AutoMode = "off"
	// AutoDownload: check and download in the background, apply on request.
	AutoDownload AutoMode = "download"
	// AutoInstall: check, download, apply and reboot in the maintenance window
	// when nobody is watching (the default, research 05 §4.2 p.8).
	AutoInstall AutoMode = "install"
)

// UnmarshalJSON accepts "off" | "download" | "install" and also true/false
// ("auto on/off": true means install).
func (m *AutoMode) UnmarshalJSON(data []byte) error {
	var b bool
	if err := json.Unmarshal(data, &b); err == nil {
		if b {
			*m = AutoInstall
		} else {
			*m = AutoOff
		}
		return nil
	}
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return errors.New("auto: want \"off\", \"download\", \"install\" or a boolean")
	}
	*m = AutoMode(s)
	return nil
}

// Settings are the user-visible knobs.
type Settings struct {
	Channel string   `json:"channel"`
	Auto    AutoMode `json:"auto"`
	// Window is the maintenance window "HH:MM-HH:MM" in the device's local
	// time; it may wrap past midnight ("23:30-01:00").
	Window string `json:"window"`
}

// Default returns the settings of a fresh device.
func Default() Settings {
	return Settings{Channel: "stable", Auto: AutoInstall, Window: "03:00-05:00"}
}

// Validate checks every field.
func (s Settings) Validate() error {
	ok := false
	for _, c := range Channels {
		if s.Channel == c {
			ok = true
		}
	}
	if !ok {
		return fmt.Errorf("channel: %q is not one of %v", s.Channel, Channels)
	}
	switch s.Auto {
	case AutoOff, AutoDownload, AutoInstall:
	default:
		return fmt.Errorf("auto: %q is not off, download or install", s.Auto)
	}
	if _, err := ParseWindow(s.Window); err != nil {
		return err
	}
	return nil
}

// Patch is a partial update (PUT /v1/settings); nil fields stay unchanged.
type Patch struct {
	Channel *string   `json:"channel,omitempty"`
	Auto    *AutoMode `json:"auto,omitempty"`
	Window  *string   `json:"window,omitempty"`
}

// Apply returns s with the patch applied (not validated).
func (s Settings) Apply(p Patch) Settings {
	if p.Channel != nil {
		s.Channel = *p.Channel
	}
	if p.Auto != nil {
		s.Auto = *p.Auto
	}
	if p.Window != nil {
		s.Window = *p.Window
	}
	return s
}

// Window is a daily time range in local time, minutes since midnight.
type Window struct {
	Start, End int
}

// ParseWindow parses "HH:MM-HH:MM".
func ParseWindow(s string) (Window, error) {
	bad := fmt.Errorf("window: %q is not HH:MM-HH:MM", s)
	if len(s) != len("00:00-00:00") || s[2] != ':' || s[5] != '-' || s[8] != ':' {
		return Window{}, bad
	}
	num := func(a string) (int, bool) {
		if a[0] < '0' || a[0] > '9' || a[1] < '0' || a[1] > '9' {
			return 0, false
		}
		return int(a[0]-'0')*10 + int(a[1]-'0'), true
	}
	sh, ok1 := num(s[0:2])
	sm, ok2 := num(s[3:5])
	eh, ok3 := num(s[6:8])
	em, ok4 := num(s[9:11])
	if !ok1 || !ok2 || !ok3 || !ok4 {
		return Window{}, bad
	}
	if sh > 23 || eh > 23 || sm > 59 || em > 59 {
		return Window{}, fmt.Errorf("window: %q has an invalid time", s)
	}
	w := Window{Start: sh*60 + sm, End: eh*60 + em}
	if w.Start == w.End {
		return Window{}, fmt.Errorf("window: %q is empty", s)
	}
	return w, nil
}

func minuteOfDay(t time.Time) int { return t.Hour()*60 + t.Minute() }

// Contains reports whether t (in its own location) is inside the window.
// The end minute is exclusive.
func (w Window) Contains(t time.Time) bool {
	m := minuteOfDay(t)
	if w.Start < w.End {
		return m >= w.Start && m < w.End
	}
	return m >= w.Start || m < w.End // wraps past midnight
}

// NextStart returns the next moment the window opens at or after t
// (t itself if the window is open now).
func (w Window) NextStart(t time.Time) time.Time {
	if w.Contains(t) {
		return t
	}
	day := time.Date(t.Year(), t.Month(), t.Day(), 0, 0, 0, 0, t.Location())
	start := day.Add(time.Duration(w.Start) * time.Minute)
	if !start.After(t) {
		start = day.AddDate(0, 0, 1).Add(time.Duration(w.Start) * time.Minute)
	}
	return start
}

// Store persists settings in a JSON file.
type Store struct {
	Path string
}

// Load reads the settings; a missing file gives the defaults. Unknown or
// broken files are an error (the daemon logs it and keeps the defaults).
func (st Store) Load() (Settings, error) {
	data, err := os.ReadFile(st.Path)
	if errors.Is(err, os.ErrNotExist) {
		return Default(), nil
	}
	if err != nil {
		return Default(), err
	}
	s := Default()
	if err := json.Unmarshal(data, &s); err != nil {
		return Default(), fmt.Errorf("%s: %w", st.Path, err)
	}
	if err := s.Validate(); err != nil {
		return Default(), fmt.Errorf("%s: %w", st.Path, err)
	}
	return s, nil
}

// Save validates and writes the settings atomically.
func (st Store) Save(s Settings) error {
	if err := s.Validate(); err != nil {
		return err
	}
	return WriteJSONAtomic(st.Path, s)
}

// WriteJSONAtomic writes v as indented JSON via a temp file, fsync and rename,
// so a power cut never leaves a half-written file.
func WriteJSONAtomic(path string, v any) error {
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	f, err := os.CreateTemp(dir, ".tmp-"+filepath.Base(path)+"-*")
	if err != nil {
		return err
	}
	tmp := f.Name()
	defer os.Remove(tmp) // no-op after a successful rename
	if _, err := f.Write(data); err != nil {
		f.Close()
		return err
	}
	if err := f.Chmod(0o600); err != nil {
		f.Close()
		return err
	}
	if err := f.Sync(); err != nil {
		f.Close()
		return err
	}
	if err := f.Close(); err != nil {
		return err
	}
	if err := os.Rename(tmp, path); err != nil {
		return err
	}
	if d, err := os.Open(dir); err == nil {
		_ = d.Sync()
		d.Close()
	}
	return nil
}
