// SPDX-License-Identifier: MPL-2.0

package tui

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
	"github.com/DenisHumen/kinolune/distro/installer/internal/install"
)

// Env is everything the TUI needs from the machine. The model never touches
// disks itself: it fills an answers.Answers and hands it to Env.Install, which
// runs the same installer as `alpha-installer install --answers` (DIST-005).
type Env interface {
	// Disks probes the disks (excluded ones included, with reasons).
	Disks() ([]disks.Disk, error)
	// Interfaces lists wired network ports.
	Interfaces() []Iface
	// Install validates a, resolves the disk and installs (or, in a dry run,
	// writes the plan). Progress lines go to log. It returns the admin's
	// yescrypt hash when the installation set one.
	Install(ctx context.Context, a *answers.Answers, log io.Writer) (hash string, err error)
	// SaveTargets lists where an answer file can be saved; exclude is the
	// install target, which is never offered.
	SaveTargets(exclude string) []SaveTarget
	// Save writes the answer file to t and returns where it went.
	Save(ctx context.Context, t SaveTarget, data []byte) (string, error)
}

// Iface is a wired network port.
type Iface struct {
	Name    string
	MAC     string
	Carrier bool
}

// ReadInterfaces lists wired ethernet ports from sysfs (root "/" on a live
// system): physical devices of type 1 that are not wireless.
func ReadInterfaces(root string) []Iface {
	base := filepath.Join(root, "sys", "class", "net")
	entries, err := os.ReadDir(base)
	if err != nil {
		return nil
	}
	var out []Iface
	for _, e := range entries {
		dir := filepath.Join(base, e.Name())
		if _, err := os.Stat(filepath.Join(dir, "device")); err != nil {
			continue // lo, bridges, veth, tun
		}
		if _, err := os.Stat(filepath.Join(dir, "wireless")); err == nil {
			continue
		}
		if _, err := os.Stat(filepath.Join(dir, "phy80211")); err == nil {
			continue
		}
		if t := readTrim(filepath.Join(dir, "type")); t != "1" {
			continue
		}
		out = append(out, Iface{
			Name:    e.Name(),
			MAC:     readTrim(filepath.Join(dir, "address")),
			Carrier: readTrim(filepath.Join(dir, "carrier")) == "1",
		})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out
}

func readTrim(p string) string {
	b, err := os.ReadFile(p)
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(b))
}

// SaveTarget is a place for the repeat-install answer file: a partition of a
// USB stick or another disk, or a file in the live system (Device empty).
type SaveTarget struct {
	Device string // /dev/sdb1; empty: Path in the live system
	FSType string
	Label  string
	Disk   string // human name of the disk
	Size   uint64
	Path   string // live-system file when Device is empty
}

// saveFS are file systems the live kernel can write and people carry answer
// files on.
var saveFS = map[string]string{
	"vfat": "vfat", "exfat": "exfat", "ext4": "ext4", "ext3": "ext3", "ext2": "ext2",
	"btrfs": "btrfs", "xfs": "xfs", "ntfs": "ntfs3",
}

// FindSaveTargets picks writable, unmounted partitions of disks that are not
// the install target, the installer's own media, optical, read-only or
// virtual. Small disks (USB sticks below the install minimum) are welcome.
func FindSaveTargets(list []disks.Disk, exclude, livePath string) []SaveTarget {
	var out []SaveTarget
	for _, d := range list {
		if d.Path == exclude || d.ReadOnly {
			continue
		}
		switch d.ExcludeReason {
		case disks.ExcludeBootMedia, disks.ExcludeVirtual, disks.ExcludeOptical, disks.ExcludeReadOnly,
			disks.ExcludeHWArea, disks.ExcludeNoMedia, disks.ExcludeNotDisk:
			continue
		}
		parts := d.Partitions
		if len(parts) == 0 && d.FSType != "" {
			parts = []disks.Partition{{Name: d.Name, Path: d.Path, Size: d.Size, FSType: d.FSType, Label: d.FSLabel}}
		}
		for _, p := range parts {
			if _, ok := saveFS[p.FSType]; !ok || len(p.Mountpoints) > 0 {
				continue
			}
			out = append(out, SaveTarget{Device: p.Path, FSType: p.FSType, Label: p.Label, Disk: d.Model, Size: p.Size})
		}
	}
	if livePath != "" {
		out = append(out, SaveTarget{Path: livePath})
	}
	return out
}

// AnswerFileName is what the text menu of the live ISO looks for on USB sticks.
const AnswerFileName = "kinolune-answers.json"

// SaveToPartition mounts t at mnt, writes the answer file and unmounts it.
func SaveToPartition(ctx context.Context, r install.Runner, t SaveTarget, mnt string, data []byte) (string, error) {
	if t.Device == "" {
		if err := os.MkdirAll(filepath.Dir(t.Path), 0o700); err != nil {
			return "", err
		}
		return t.Path, os.WriteFile(t.Path, data, 0o600)
	}
	fs, ok := saveFS[t.FSType]
	if !ok {
		return "", fmt.Errorf("%s: unsupported file system %q", t.Device, t.FSType)
	}
	if err := os.MkdirAll(mnt, 0o700); err != nil {
		return "", err
	}
	if _, err := r.Run(ctx, install.Cmd{Name: "mount", Args: []string{"-t", fs, t.Device, mnt}}); err != nil {
		return "", err
	}
	werr := os.WriteFile(filepath.Join(mnt, AnswerFileName), data, 0o600)
	_, serr := r.Run(ctx, install.Cmd{Name: "sync"})
	_, uerr := r.Run(ctx, install.Cmd{Name: "umount", Args: []string{mnt}})
	if err := errors.Join(werr, serr, uerr); err != nil {
		return "", err
	}
	return t.Device + ":/" + AnswerFileName, nil
}
