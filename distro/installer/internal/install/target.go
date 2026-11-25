// SPDX-License-Identifier: MPL-2.0

package install

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
)

// ResolveTarget finds the disk named by the answer file among the probed
// disks and refuses everything that must never be wiped: the disk the
// installer runs from, excluded devices, partitions, disks with mounted
// file systems. resolve follows /dev/disk/by-* symlinks (filepath.EvalSymlinks
// on a live system).
func ResolveTarget(path string, list []disks.Disk, resolve func(string) (string, error)) (disks.Disk, error) {
	real := path
	if resolve != nil {
		r, err := resolve(path)
		if err != nil {
			return disks.Disk{}, fmt.Errorf("disk %s: %w", path, err)
		}
		real = r
	}
	real = filepath.Clean(real)
	for _, d := range list {
		if d.Path != real && "/dev/"+d.Name != real {
			continue
		}
		if d.Excluded {
			return disks.Disk{}, fmt.Errorf("refusing to install to %s: %s", real, excludeText(d.ExcludeReason))
		}
		for _, f := range d.Findings {
			if f.Kind == disks.FindingMounted {
				return disks.Disk{}, fmt.Errorf("refusing to install to %s: %s is mounted at %s; unmount it first",
					real, f.Device, f.Detail)
			}
		}
		if d.Size < MinDiskSize+gptOverhead {
			return disks.Disk{}, fmt.Errorf("refusing to install to %s: %s is smaller than the minimum %s",
				real, d.SizeString(), decimal(MinDiskSize+gptOverhead))
		}
		return d, nil
	}
	for _, d := range list {
		for _, p := range d.Partitions {
			if p.Path == real {
				return disks.Disk{}, fmt.Errorf("%s is a partition of %s; give the whole disk", real, d.Path)
			}
		}
	}
	return disks.Disk{}, fmt.Errorf("%s is not a disk this installer can see (see `alpha-installer disks --all`)", real)
}

func excludeText(r disks.ExcludeReason) string {
	switch r {
	case disks.ExcludeBootMedia:
		return "the installer (or the running system) boots from this disk"
	case disks.ExcludeVirtual:
		return "it is a virtual device (loop, zram, nbd)"
	case disks.ExcludeOptical:
		return "it is an optical drive"
	case disks.ExcludeNoMedia:
		return "there is no medium in it"
	case disks.ExcludeReadOnly:
		return "it is read-only"
	case disks.ExcludeTooSmall:
		return "it is too small"
	case disks.ExcludeHWArea:
		return "it is a hardware boot area, not a disk"
	}
	return strings.ReplaceAll(string(r), "-", " ")
}
