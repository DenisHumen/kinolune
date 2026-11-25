// SPDX-License-Identifier: MPL-2.0

package install

import (
	"fmt"
	"runtime"
	"strings"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
)

// GiB is 2^30 bytes: partitions are sized in binary units, disks are shown
// in decimal ones (as on the box).
const GiB = uint64(1) << 30

// Fixed partition sizes (05 §2.4).
const (
	ESPSize  = 1 * GiB
	BootSize = 2 * GiB
	// gptOverhead covers the 1 MiB alignment gap at the start and the backup
	// GPT at the end of the disk.
	gptOverhead = 2 << 20
)

// MinDiskSize is the smallest disk the installer accepts: ESP + /boot + the
// smallest root (19 GiB, about 20.4 GB with the GPT overhead). DIST-004 used 16 GB provisionally.
const MinDiskSize = ESPSize + BootSize + uint64(answers.MinRootSizeGiB)*GiB

// Partition labels and file-system labels (the same string for both).
const (
	LabelESP  = "EFI-SYSTEM"
	LabelBoot = "boot"
	LabelRoot = "root"
	LabelData = "kinolune-data"
)

// VarSubvolume is the btrfs subvolume mounted at /var. bootc rolls back /usr
// and merges /etc but never touches /var, so DIST-010 snapshots this
// subvolume before every update.
const VarSubvolume = "@var"

// DataMountBase is where data partitions are mounted: /srv is a symlink to
// var/srv on bootc systems, fstab needs the real path.
const DataMountBase = "/var/srv/data"

// GPT partition type GUIDs (Discoverable Partitions Specification).
const (
	typeESP       = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B"
	typeLinuxData = "0FC63DAF-8483-4772-8E79-3D69D8477DE4"
)

// rootType is the architecture-specific root partition type.
func rootType(goarch string) string {
	switch goarch {
	case "arm64":
		return "B921B045-1DF0-41C3-AF44-4C6F280D3FAE"
	default: // amd64
		return "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709"
	}
}

// Part is one partition of the plan.
type Part struct {
	Number     int
	Label      string // GPT partition name and file-system label
	TypeGUID   string
	Size       uint64 // 0 = the rest of the disk
	FS         string // vfat, ext4, btrfs
	Mountpoint string // in the installed system
	Device     string // /dev/nvme0n1p3
}

// Layout is the partition plan for one disk.
type Layout struct {
	Disk     string // /dev/nvme0n1
	DiskSize uint64
	Parts    []Part
}

// PartitionPath returns the device node of partition n of disk:
// sda → sda3, nvme0n1 / mmcblk0 / loop0 → …p3 (name ends in a digit).
func PartitionPath(disk string, n int) string {
	if disk != "" && disk[len(disk)-1] >= '0' && disk[len(disk)-1] <= '9' {
		return fmt.Sprintf("%sp%d", disk, n)
	}
	return fmt.Sprintf("%s%d", disk, n)
}

// PlanLayout computes the GPT layout for a disk of diskSize bytes.
func PlanLayout(disk string, diskSize uint64, d answers.Disk) (Layout, error) {
	return planLayout(disk, diskSize, d, runtime.GOARCH)
}

func planLayout(disk string, diskSize uint64, d answers.Disk, goarch string) (Layout, error) {
	if diskSize < MinDiskSize+gptOverhead {
		return Layout{}, fmt.Errorf("%s is too small: %s, the system needs at least %s",
			disk, decimal(diskSize), decimal(MinDiskSize+gptOverhead))
	}
	l := Layout{Disk: disk, DiskSize: diskSize}
	l.Parts = []Part{
		{Number: 1, Label: LabelESP, TypeGUID: typeESP, Size: ESPSize, FS: "vfat", Mountpoint: "/boot/efi"},
		{Number: 2, Label: LabelBoot, TypeGUID: typeLinuxData, Size: BootSize, FS: "ext4", Mountpoint: "/boot"},
		{Number: 3, Label: LabelRoot, TypeGUID: rootType(goarch), FS: "btrfs", Mountpoint: "/"},
	}
	rest := diskSize - ESPSize - BootSize - gptOverhead
	if d.DataPartition {
		root := uint64(d.RootSizeGiB) * GiB
		minData := uint64(answers.MinDataSizeGiB) * GiB
		if rest < root+minData {
			return Layout{}, fmt.Errorf("%s (%s) has no room for a %d GiB root and a data partition of at least %d GiB; "+
				"lower disk.root_size_gib or turn off disk.data_partition",
				disk, decimal(diskSize), d.RootSizeGiB, answers.MinDataSizeGiB)
		}
		l.Parts[2].Size = root
		l.Parts = append(l.Parts, Part{
			Number: 4, Label: LabelData, TypeGUID: typeLinuxData, FS: "btrfs",
			Mountpoint: DataMountBase + "/" + d.DataPool,
		})
	}
	for i := range l.Parts {
		l.Parts[i].Device = PartitionPath(disk, l.Parts[i].Number)
	}
	return l, nil
}

// SizeOf is the size of a partition; for the "rest" partition it is the
// estimate the plan prints.
func (l Layout) SizeOf(p Part) uint64 {
	if p.Size != 0 {
		return p.Size
	}
	used := uint64(gptOverhead)
	for _, q := range l.Parts {
		used += q.Size
	}
	return l.DiskSize - used
}

// Part returns the partition with the given label.
func (l Layout) Part(label string) (Part, bool) {
	for _, p := range l.Parts {
		if p.Label == label {
			return p, true
		}
	}
	return Part{}, false
}

// SfdiskScript is the sfdisk(8) input that creates the layout.
func (l Layout) SfdiskScript() string {
	var b strings.Builder
	b.WriteString("label: gpt\n")
	for _, p := range l.Parts {
		fmt.Fprintf(&b, "%s : ", PartitionPath(l.Disk, p.Number))
		if p.Size != 0 {
			fmt.Fprintf(&b, "size=%dMiB, ", p.Size>>20)
		}
		fmt.Fprintf(&b, "type=%s, name=%q\n", p.TypeGUID, p.Label)
	}
	return b.String()
}

func decimal(b uint64) string {
	switch {
	case b >= 1_000_000_000_000:
		return fmt.Sprintf("%.1f TB", float64(b)/1e12)
	default:
		return fmt.Sprintf("%.1f GB", float64(b)/1e9)
	}
}

func binary(b uint64) string {
	if b%GiB == 0 {
		return fmt.Sprintf("%d GiB", b/GiB)
	}
	return fmt.Sprintf("%.1f GiB", float64(b)/float64(GiB))
}
