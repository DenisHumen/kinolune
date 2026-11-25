// SPDX-License-Identifier: MPL-2.0

package system

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"slices"
	"strconv"
	"time"

	"github.com/godbus/dbus/v5"
)

const (
	systemdDest  = "org.freedesktop.systemd1"
	systemdPath  = dbus.ObjectPath("/org/freedesktop/systemd1")
	systemdMgr   = "org.freedesktop.systemd1.Manager"
	systemdUnit  = "org.freedesktop.systemd1.Unit"
	systemdSvc   = "org.freedesktop.systemd1.Service"
	journalLimit = 1000
)

// Unit is a unit the admin UI shows.
type Unit struct {
	Name string `json:"name"`
	// Title is a human name for the UI (Russian; the UI translates by name).
	Title string `json:"title"`
	// Restartable: the restart button is offered.
	Restartable bool `json:"restartable"`
}

// OurUnits are the services of the NAS OS the admin UI shows. Only these can
// be queried, restarted or have their journal read: the polkit rule allows
// manage-units for exactly the restartable ones.
var OurUnits = []Unit{
	{"alpha-media.service", "Медиасервер", true},
	{"alpha-admin.service", "Админка", true},
	{"alpha-updated.service", "Обновления", true},
	{"smb.service", "Общие папки (SMB)", true},
	{"wsdd.service", "Обнаружение в сети Windows", true},
	{"avahi-daemon.service", "Обнаружение в сети (mDNS)", true},
	{"smartd.service", "Мониторинг дисков (SMART)", true},
	{"chronyd.service", "Время", true},
	{"NetworkManager.service", "Сеть", false},
	{"firewalld.service", "Файрвол", false},
	{"sshd.service", "SSH", false},
	{"cockpit.socket", "Расширенный режим (Cockpit)", false},
}

func findUnit(name string) (Unit, bool) {
	i := slices.IndexFunc(OurUnits, func(u Unit) bool { return u.Name == name })
	if i < 0 {
		return Unit{}, false
	}
	return OurUnits[i], true
}

// UnitStatus is the state of one unit.
type UnitStatus struct {
	Unit
	Description   string     `json:"description"`
	LoadState     string     `json:"load_state"`
	ActiveState   string     `json:"active_state"`
	SubState      string     `json:"sub_state"`
	UnitFileState string     `json:"unit_file_state"`
	Since         *time.Time `json:"since,omitempty"`
	MainPID       uint32     `json:"main_pid,omitempty"`
	Restarts      uint32     `json:"restarts,omitempty"`
	Result        string     `json:"result,omitempty"`
}

// JournalEntry is one log line.
type JournalEntry struct {
	Time     time.Time `json:"time"`
	Priority int       `json:"priority"`
	PID      int       `json:"pid,omitempty"`
	Message  string    `json:"message"`
}

// Systemd reads unit state and restarts units over D-Bus; the journal is
// read with journalctl (journald has no D-Bus API for reading).
type Systemd struct {
	Bus *Bus
	// Journalctl is the journalctl binary.
	Journalctl string
}

// Units returns the status of all our units.
func (s *Systemd) Units(ctx context.Context) ([]UnitStatus, error) {
	out := make([]UnitStatus, 0, len(OurUnits))
	for _, u := range OurUnits {
		st, err := s.status(ctx, u)
		if err != nil {
			return nil, err
		}
		out = append(out, st)
	}
	return out, nil
}

// Unit returns the status of one of our units.
func (s *Systemd) Unit(ctx context.Context, name string) (UnitStatus, error) {
	u, ok := findUnit(name)
	if !ok {
		return UnitStatus{}, errf(ErrNotFound, "unit %q", name)
	}
	return s.status(ctx, u)
}

func (s *Systemd) status(ctx context.Context, u Unit) (UnitStatus, error) {
	var path dbus.ObjectPath
	// LoadUnit (not GetUnit) so units that are not running still report.
	if err := s.Bus.call(ctx, systemdDest, systemdPath, systemdMgr+".LoadUnit", []any{&path}, u.Name); err != nil {
		return UnitStatus{}, fmt.Errorf("load %s: %w", u.Name, err)
	}
	p, err := s.Bus.getAll(ctx, systemdDest, path, systemdUnit)
	if err != nil {
		return UnitStatus{}, fmt.Errorf("%s: %w", u.Name, err)
	}
	st := UnitStatus{
		Unit:          u,
		Description:   str(p, "Description"),
		LoadState:     str(p, "LoadState"),
		ActiveState:   str(p, "ActiveState"),
		SubState:      str(p, "SubState"),
		UnitFileState: str(p, "UnitFileState"),
	}
	if st.ActiveState == "active" {
		st.Since = usecTime(u64(p, "ActiveEnterTimestamp"))
	} else {
		st.Since = usecTime(u64(p, "InactiveEnterTimestamp"))
	}
	if st.LoadState == "loaded" && len(u.Name) > 8 && u.Name[len(u.Name)-8:] == ".service" {
		if sp, err := s.Bus.getAll(ctx, systemdDest, path, systemdSvc); err == nil {
			st.MainPID = u32(sp, "MainPID")
			st.Restarts = u32(sp, "NRestarts")
			st.Result = str(sp, "Result")
		}
	}
	return st, nil
}

// Restart restarts one of our restartable units and returns the job path.
func (s *Systemd) Restart(ctx context.Context, name string) (string, error) {
	u, ok := findUnit(name)
	if !ok {
		return "", errf(ErrNotFound, "unit %q", name)
	}
	if !u.Restartable {
		return "", errf(ErrForbidden, "unit %q cannot be restarted from the admin UI", name)
	}
	var job dbus.ObjectPath
	if err := s.Bus.call(ctx, systemdDest, systemdPath, systemdMgr+".RestartUnit", []any{&job}, name, "replace"); err != nil {
		return "", fmt.Errorf("restart %s: %w", name, err)
	}
	return string(job), nil
}

// Journal returns the last n lines of one of our units, oldest first.
func (s *Systemd) Journal(ctx context.Context, name string, n int) ([]JournalEntry, error) {
	if _, ok := findUnit(name); !ok {
		return nil, errf(ErrNotFound, "unit %q", name)
	}
	if n <= 0 {
		n = 100
	}
	n = min(n, journalLimit)
	bin := s.Journalctl
	if bin == "" {
		bin = "journalctl"
	}
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, bin, "--unit="+name, "--lines="+strconv.Itoa(n), "--output=json",
		"--no-pager", "--quiet", "--output-fields=MESSAGE,PRIORITY,_PID,__REALTIME_TIMESTAMP")
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("journalctl: %v: %s", err, bytes.TrimSpace(stderr.Bytes()))
	}
	return parseJournal(out)
}

func parseJournal(out []byte) ([]JournalEntry, error) {
	entries := []JournalEntry{}
	sc := bufio.NewScanner(bytes.NewReader(out))
	sc.Buffer(make([]byte, 0, 64<<10), 4<<20)
	for sc.Scan() {
		line := bytes.TrimSpace(sc.Bytes())
		if len(line) == 0 {
			continue
		}
		var raw map[string]json.RawMessage
		if err := json.Unmarshal(line, &raw); err != nil {
			return nil, fmt.Errorf("journal: %w", err)
		}
		e := JournalEntry{Priority: 6, Message: journalString(raw["MESSAGE"])}
		if us, err := strconv.ParseInt(journalString(raw["__REALTIME_TIMESTAMP"]), 10, 64); err == nil {
			e.Time = time.UnixMicro(us).UTC()
		}
		if p, err := strconv.Atoi(journalString(raw["PRIORITY"])); err == nil {
			e.Priority = p
		}
		if pid, err := strconv.Atoi(journalString(raw["_PID"])); err == nil {
			e.PID = pid
		}
		entries = append(entries, e)
	}
	return entries, sc.Err()
}

// journalString decodes a journal JSON field: a string, or an array of bytes
// for binary/non-UTF-8 messages.
func journalString(r json.RawMessage) string {
	if len(r) == 0 {
		return ""
	}
	var s string
	if json.Unmarshal(r, &s) == nil {
		return s
	}
	var b []byte
	var ints []int
	if json.Unmarshal(r, &ints) == nil {
		b = make([]byte, len(ints))
		for i, v := range ints {
			b[i] = byte(v)
		}
		return string(bytes.ToValidUTF8(b, []byte("�")))
	}
	return ""
}
