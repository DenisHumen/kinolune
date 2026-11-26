// SPDX-License-Identifier: MPL-2.0

// Package varsnap keeps read-only btrfs snapshots of the @var subvolume and
// puts /var back to the pre-update snapshot when the OS update did not hold
// (DIST-010, ADR-0164, research 05 §4.2 p.6).
//
// bootc rolls back /usr and merges /etc, but /var (media server database,
// app state) stays as the new version left it. So right before rebooting into
// an update the agent takes a read-only snapshot of @var and writes a marker
// (pending.json). Early in every boot, before var.mount, `alpha-updated
// var-restore` looks at the marker:
//
//   - booted the new deployment: remember that it booted (to_booted);
//   - booted the old deployment after the new one ran (greenboot rollback or a
//     rollback within the confirmation window): rename the current @var aside
//     (kept as a read-only "replaced" snapshot, nothing is lost) and make @var a
//     writable snapshot of the pre-update one;
//   - booted the old deployment and the new one never ran (the reboot did not
//     happen): drop the marker, /var was not touched by the new version.
//
// The agent drops the marker once the new deployment is healthy
// (boot-complete.target, reached after greenboot's required checks), so a
// later manual OS rollback never throws away weeks of state: the database is
// then opened by the old version thanks to expand→contract migrations (LIB-002).
//
// Layout on the root btrfs (top level, subvolid=5):
//
//	@var                      mounted at /var (fstab: subvol=/@var)
//	@var-snapshots/           snapshots of @var and the markers
//	  20261126T030000Z        read-only, pre-update
//	  20261126T030000Z.json   its metadata
//	  pending.json            marker of the update being tried
//	  last-restore.json       what the last restore did
package varsnap

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"log/slog"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
)

// ErrNoLayout: /var is not a btrfs subvolume of the root filesystem (a
// development disk without the installer layout); snapshots are skipped.
var ErrNoLayout = errors.New("/var is not a btrfs subvolume of the root filesystem")

// Kinds of snapshots.
const (
	KindPreUpdate = "pre-update"
	KindReplaced  = "replaced" // @var as it was before a restore
)

const (
	dirSuffix       = "-snapshots"
	pendingFile     = "pending.json"
	lastRestoreFile = "last-restore.json"
	nameLayout      = "20060102T150405Z"
)

// Btrfs does the filesystem work; Exec runs the real tools, tests use a fake.
type Btrfs interface {
	// MountTop mounts the top level (subvolid=5) of the filesystem on dev at dir.
	MountTop(ctx context.Context, dev, dir string) error
	Unmount(ctx context.Context, dir string) error
	// Snapshot makes dst a snapshot of the subvolume src.
	Snapshot(ctx context.Context, src, dst string, readonly bool) error
	// SetReadOnly makes a subvolume read-only.
	SetReadOnly(ctx context.Context, path string) error
	Delete(ctx context.Context, path string) error
}

// Exec runs mount, umount and btrfs.
type Exec struct{}

func run(ctx context.Context, name string, args ...string) error {
	out, err := exec.CommandContext(ctx, name, args...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("%s %s: %w: %s", name, strings.Join(args, " "), err, strings.TrimSpace(string(out)))
	}
	return nil
}

func (Exec) MountTop(ctx context.Context, dev, dir string) error {
	return run(ctx, "mount", "-t", "btrfs", "-o", "subvolid=5,noatime", dev, dir)
}
func (Exec) Unmount(ctx context.Context, dir string) error { return run(ctx, "umount", dir) }
func (Exec) Snapshot(ctx context.Context, src, dst string, readonly bool) error {
	args := []string{"subvolume", "snapshot"}
	if readonly {
		args = append(args, "-r")
	}
	return run(ctx, "btrfs", append(args, src, dst)...)
}
func (Exec) SetReadOnly(ctx context.Context, path string) error {
	return run(ctx, "btrfs", "property", "set", "-ts", path, "ro", "true")
}
func (Exec) Delete(ctx context.Context, path string) error {
	return run(ctx, "btrfs", "subvolume", "delete", path)
}

// Pending is the marker of an update being tried.
type Pending struct {
	Snapshot     string    `json:"snapshot"`
	Created      time.Time `json:"created"`
	FromChecksum string    `json:"from_checksum"`
	FromVersion  string    `json:"from_version,omitempty"`
	ToChecksum   string    `json:"to_checksum"`
	ToDigest     string    `json:"to_digest,omitempty"`
	ToVersion    string    `json:"to_version,omitempty"`
	// ToBooted: the new deployment has booted at least once since the snapshot.
	ToBooted bool `json:"to_booted"`
}

// Snapshot is one snapshot of @var.
type Snapshot struct {
	Name        string    `json:"name"`
	Kind        string    `json:"kind"`
	Created     time.Time `json:"created"`
	FromVersion string    `json:"from_version,omitempty"`
	ToVersion   string    `json:"to_version,omitempty"`
}

// Restore records what the last restore did.
type Restore struct {
	At       time.Time `json:"at"`
	Snapshot string    `json:"snapshot"`
	Replaced string    `json:"replaced"`
	// Version that did not hold (to_version of the marker).
	Version string `json:"version,omitempty"`
}

// State is what the agent shows in its status.
type State struct {
	Snapshots   []Snapshot `json:"snapshots"`
	Pending     *Pending   `json:"pending,omitempty"`
	LastRestore *Restore   `json:"last_restore,omitempty"`
}

// Manager works on the root btrfs. Methods mount the top level for the
// duration of one call; they are not safe for concurrent use.
type Manager struct {
	// Fstab gives the @var subvolume (default /etc/fstab).
	Fstab string
	// Mountinfo and Sysroot give the root device: the filesystem mounted at
	// Sysroot (default /proc/self/mountinfo and /sysroot).
	Mountinfo string
	Sysroot   string
	// MountDir is where the top level is mounted (default /run/alpha-varsnap).
	MountDir string
	// Keep is how many snapshots stay (default 3).
	Keep int
	FS   Btrfs
	Now  func() time.Time
	Log  *slog.Logger
}

func (m *Manager) defaults() {
	if m.Fstab == "" {
		m.Fstab = "/etc/fstab"
	}
	if m.Mountinfo == "" {
		m.Mountinfo = "/proc/self/mountinfo"
	}
	if m.Sysroot == "" {
		m.Sysroot = "/sysroot"
	}
	if m.MountDir == "" {
		m.MountDir = "/run/alpha-varsnap"
	}
	if m.Keep <= 0 {
		m.Keep = 3
	}
	if m.FS == nil {
		m.FS = Exec{}
	}
	if m.Now == nil {
		m.Now = time.Now
	}
	if m.Log == nil {
		m.Log = slog.New(slog.DiscardHandler)
	}
}

// layout finds the root device and the @var subvolume name.
func (m *Manager) layout() (dev, subvol string, err error) {
	subvol, err = VarSubvol(m.Fstab)
	if err != nil {
		return "", "", err
	}
	dev, err = MountSource(m.Mountinfo, m.Sysroot, "btrfs")
	if err != nil {
		return "", "", err
	}
	return dev, subvol, nil
}

// withTop mounts the top level, runs fn with the paths of @var and of the
// snapshots directory, and unmounts.
func (m *Manager) withTop(ctx context.Context, fn func(varPath, snapDir string) error) (err error) {
	m.defaults()
	dev, subvol, err := m.layout()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(m.MountDir, 0o700); err != nil {
		return err
	}
	if err := m.FS.MountTop(ctx, dev, m.MountDir); err != nil {
		return err
	}
	defer func() {
		if uerr := m.FS.Unmount(context.WithoutCancel(ctx), m.MountDir); uerr != nil && err == nil {
			err = uerr
		}
	}()
	varPath := filepath.Join(m.MountDir, subvol)
	if _, err := os.Stat(varPath); err != nil {
		return fmt.Errorf("%w: %v", ErrNoLayout, err)
	}
	return fn(varPath, varPath+dirSuffix)
}

// Prepare snapshots @var read-only and writes the marker p (its Snapshot and
// Created are filled in). Old snapshots beyond Keep are deleted.
func (m *Manager) Prepare(ctx context.Context, p Pending) (Pending, error) {
	err := m.withTop(ctx, func(varPath, snapDir string) error {
		if err := os.MkdirAll(snapDir, 0o700); err != nil {
			return err
		}
		now := m.Now().UTC()
		p.Created = now
		p.Snapshot = uniqueName(snapDir, now.Format(nameLayout))
		p.ToBooted = false
		if err := m.FS.Snapshot(ctx, varPath, filepath.Join(snapDir, p.Snapshot), true); err != nil {
			return err
		}
		meta := Snapshot{Name: p.Snapshot, Kind: KindPreUpdate, Created: now, FromVersion: p.FromVersion, ToVersion: p.ToVersion}
		if err := writeJSON(filepath.Join(snapDir, p.Snapshot+".json"), meta); err != nil {
			return err
		}
		if err := writeJSON(filepath.Join(snapDir, pendingFile), p); err != nil {
			return err
		}
		m.Log.Info("/var snapshot taken", "snapshot", p.Snapshot, "from", p.FromVersion, "to", p.ToVersion)
		m.prune(ctx, snapDir, p.Snapshot)
		return nil
	})
	return p, err
}

// Confirm drops the marker: the update is healthy, /var stays as it is.
func (m *Manager) Confirm(ctx context.Context) error {
	return m.withTop(ctx, func(_, snapDir string) error {
		err := os.Remove(filepath.Join(snapDir, pendingFile))
		if errors.Is(err, fs.ErrNotExist) {
			return nil
		}
		return err
	})
}

// State lists snapshots and markers.
func (m *Manager) State(ctx context.Context) (State, error) {
	var st State
	err := m.withTop(ctx, func(_, snapDir string) error {
		var err error
		st, err = readState(snapDir)
		return err
	})
	return st, err
}

// BootAction is what Boot did.
type BootAction string

const (
	BootNothing   BootAction = "nothing"   // no marker
	BootToBooted  BootAction = "to-booted" // the new deployment boots
	BootDropped   BootAction = "dropped"   // marker dropped, /var untouched
	BootRestored  BootAction = "restored"  // /var put back to the snapshot
	BootUnchanged BootAction = "unchanged" // marker kept as it is
)

// Boot runs early in boot, before /var is mounted, with the ostree checksum
// of the booted deployment.
func (m *Manager) Boot(ctx context.Context, booted string) (BootAction, error) {
	act := BootNothing
	err := m.withTop(ctx, func(varPath, snapDir string) error {
		var p Pending
		switch err := readJSON(filepath.Join(snapDir, pendingFile), &p); {
		case errors.Is(err, fs.ErrNotExist):
			return nil
		case err != nil:
			return err
		}
		switch {
		case booted == "":
			act = BootUnchanged
			return nil
		case booted == p.ToChecksum:
			act = BootUnchanged
			if !p.ToBooted {
				p.ToBooted = true
				act = BootToBooted
				return writeJSON(filepath.Join(snapDir, pendingFile), p)
			}
			return nil
		case booted == p.FromChecksum && p.ToBooted:
			if err := m.restore(ctx, varPath, snapDir, p); err != nil {
				return err
			}
			act = BootRestored
			return nil
		default:
			// The update never ran (or another deployment booted): /var is
			// as the running version knows it.
			act = BootDropped
			m.Log.Info("update did not boot, /var snapshot marker dropped", "snapshot", p.Snapshot)
			return os.Remove(filepath.Join(snapDir, pendingFile))
		}
	})
	return act, err
}

func (m *Manager) restore(ctx context.Context, varPath, snapDir string, p Pending) error {
	src := filepath.Join(snapDir, p.Snapshot)
	if _, err := os.Stat(src); err != nil {
		return fmt.Errorf("snapshot %s: %w", p.Snapshot, err)
	}
	now := m.Now().UTC()
	replaced := uniqueName(snapDir, now.Format(nameLayout)+"-"+KindReplaced)
	aside := filepath.Join(snapDir, replaced)
	// Move the current @var aside (a rename keeps it intact), then make a
	// writable @var from the pre-update snapshot. If that fails, put it back.
	if err := os.Rename(varPath, aside); err != nil {
		return err
	}
	if err := m.FS.Snapshot(ctx, src, varPath, false); err != nil {
		if rerr := os.Rename(aside, varPath); rerr != nil {
			return fmt.Errorf("%w; and could not put @var back: %v", err, rerr)
		}
		return err
	}
	if err := m.FS.SetReadOnly(ctx, aside); err != nil {
		m.Log.Warn("cannot make the replaced /var read-only", "path", replaced, "err", err)
	}
	meta := Snapshot{Name: replaced, Kind: KindReplaced, Created: now, FromVersion: p.ToVersion, ToVersion: p.FromVersion}
	if err := writeJSON(filepath.Join(snapDir, replaced+".json"), meta); err != nil {
		m.Log.Warn("cannot write snapshot metadata", "err", err)
	}
	rec := Restore{At: now, Snapshot: p.Snapshot, Replaced: replaced, Version: p.ToVersion}
	if err := writeJSON(filepath.Join(snapDir, lastRestoreFile), rec); err != nil {
		m.Log.Warn("cannot write restore record", "err", err)
	}
	if err := os.Remove(filepath.Join(snapDir, pendingFile)); err != nil {
		return err
	}
	m.Log.Warn("/var restored to the pre-update snapshot", "snapshot", p.Snapshot, "replaced", replaced, "version", p.ToVersion)
	m.prune(ctx, snapDir, p.Snapshot)
	return nil
}

// prune deletes the oldest snapshots beyond Keep, never keep.
func (m *Manager) prune(ctx context.Context, snapDir, keep string) {
	snaps, err := listSnapshots(snapDir)
	if err != nil {
		m.Log.Warn("cannot list /var snapshots", "err", err)
		return
	}
	left := len(snaps)
	for _, s := range snaps {
		if left <= m.Keep {
			break
		}
		if s.Name == keep {
			continue
		}
		if err := m.FS.Delete(ctx, filepath.Join(snapDir, s.Name)); err != nil {
			m.Log.Warn("cannot delete old /var snapshot", "snapshot", s.Name, "err", err)
			continue
		}
		_ = os.Remove(filepath.Join(snapDir, s.Name+".json"))
		left--
		m.Log.Info("old /var snapshot deleted", "snapshot", s.Name)
	}
}

// listSnapshots returns snapshots oldest first (names sort by time).
func listSnapshots(snapDir string) ([]Snapshot, error) {
	ents, err := os.ReadDir(snapDir)
	if errors.Is(err, fs.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var out []Snapshot
	for _, e := range ents {
		if !e.IsDir() || strings.HasPrefix(e.Name(), ".") {
			continue
		}
		s := Snapshot{Name: e.Name(), Kind: KindPreUpdate}
		_ = readJSON(filepath.Join(snapDir, e.Name()+".json"), &s)
		s.Name = e.Name()
		out = append(out, s)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out, nil
}

func readState(snapDir string) (State, error) {
	var st State
	snaps, err := listSnapshots(snapDir)
	if err != nil {
		return st, err
	}
	st.Snapshots = snaps
	var p Pending
	if err := readJSON(filepath.Join(snapDir, pendingFile), &p); err == nil {
		st.Pending = &p
	} else if !errors.Is(err, fs.ErrNotExist) {
		return st, err
	}
	var r Restore
	if err := readJSON(filepath.Join(snapDir, lastRestoreFile), &r); err == nil {
		st.LastRestore = &r
	}
	return st, nil
}

func uniqueName(dir, base string) string {
	name := base
	for i := 2; ; i++ {
		if _, err := os.Lstat(filepath.Join(dir, name)); errors.Is(err, fs.ErrNotExist) {
			return name
		}
		name = fmt.Sprintf("%s-%d", base, i)
	}
}

func readJSON(path string, v any) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	return json.Unmarshal(data, v)
}

// writeJSON writes atomically (temp, fsync, rename).
func writeJSON(path string, v any) error {
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(path), ".tmp-*")
	if err != nil {
		return err
	}
	tmp := f.Name()
	defer os.Remove(tmp)
	if _, err := f.Write(append(data, '\n')); err != nil {
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
	return os.Rename(tmp, path)
}

// VarSubvol returns the subvolume of the btrfs /var line in fstab ("@var").
func VarSubvol(fstab string) (string, error) {
	f, err := os.Open(fstab)
	if err != nil {
		return "", fmt.Errorf("%w: %v", ErrNoLayout, err)
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		fields := strings.Fields(sc.Text())
		if len(fields) < 4 || strings.HasPrefix(fields[0], "#") || fields[1] != "/var" {
			continue
		}
		if fields[2] != "btrfs" {
			return "", ErrNoLayout
		}
		for _, o := range strings.Split(fields[3], ",") {
			if v, ok := strings.CutPrefix(o, "subvol="); ok {
				v = strings.Trim(v, "/")
				if v == "" || strings.Contains(v, "/") || strings.HasPrefix(v, ".") {
					return "", fmt.Errorf("%w: unsupported subvol=%s", ErrNoLayout, v)
				}
				return v, nil
			}
		}
		return "", fmt.Errorf("%w: no subvol= for /var", ErrNoLayout)
	}
	if err := sc.Err(); err != nil {
		return "", err
	}
	return "", ErrNoLayout
}

// MountSource returns the source device of the filesystem of type fstype
// mounted at target, from a mountinfo file.
func MountSource(mountinfo, target, fstype string) (string, error) {
	data, err := os.ReadFile(mountinfo)
	if err != nil {
		return "", err
	}
	for _, line := range strings.Split(string(data), "\n") {
		pre, post, ok := strings.Cut(line, " - ")
		if !ok {
			continue
		}
		a, b := strings.Fields(pre), strings.Fields(post)
		if len(a) < 5 || len(b) < 2 || unescape(a[4]) != target {
			continue
		}
		if b[0] != fstype {
			return "", fmt.Errorf("%w: %s is %s", ErrNoLayout, target, b[0])
		}
		return unescape(b[1]), nil
	}
	return "", fmt.Errorf("%w: %s is not mounted", ErrNoLayout, target)
}

func unescape(s string) string {
	if !strings.Contains(s, `\`) {
		return s
	}
	var b strings.Builder
	for i := 0; i < len(s); i++ {
		if s[i] == '\\' && i+3 < len(s) {
			if c, err := strconv.ParseUint(s[i+1:i+4], 8, 8); err == nil {
				b.WriteByte(byte(c))
				i += 3
				continue
			}
		}
		b.WriteByte(s[i])
	}
	return b.String()
}

// BootedChecksum returns the ostree checksum of the booted deployment from
// the kernel command line (ostree=/ostree/boot.N/<stateroot>/<bootcsum>/<i>,
// a symlink chain into <sysroot>/ostree/deploy/<stateroot>/deploy/<checksum>.<serial>).
func BootedChecksum(cmdline, sysroot string) (string, error) {
	data, err := os.ReadFile(cmdline)
	if err != nil {
		return "", err
	}
	for _, f := range strings.Fields(string(data)) {
		arg, ok := strings.CutPrefix(f, "ostree=")
		if !ok {
			continue
		}
		dst, err := filepath.EvalSymlinks(filepath.Join(sysroot, arg))
		if err != nil {
			return "", err
		}
		base := filepath.Base(dst)
		if i := strings.LastIndex(base, "."); i > 0 {
			return base[:i], nil
		}
		return "", fmt.Errorf("unexpected deployment path %s", dst)
	}
	return "", errors.New("no ostree= on the kernel command line")
}
