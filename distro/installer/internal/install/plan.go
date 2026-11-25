// SPDX-License-Identifier: MPL-2.0

package install

import (
	"fmt"
	"io"
	"strings"
	"text/tabwriter"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
)

// WritePlan prints what Run would do. It touches nothing: this is the
// output of `alpha-installer install --dry-run` and the text shown before
// the confirmation.
func (in *Installer) WritePlan(w io.Writer) error {
	in.defaults()
	a := in.Answers
	d := in.Disk
	fmt.Fprintf(w, "Kinolune OS installation plan\n\n")
	fmt.Fprintf(w, "Disk:     %s, %s", d.Label(), d.SizeString())
	if d.Serial != "" {
		fmt.Fprintf(w, ", S/N %s", d.Serial)
	}
	fmt.Fprintln(w)
	if d.HasData() || len(d.Partitions) > 0 {
		fmt.Fprintln(w, "          EVERYTHING ON THIS DISK WILL BE ERASED:")
		for _, f := range d.Findings {
			fmt.Fprintf(w, "            - %s\n", f)
		}
	} else {
		fmt.Fprintln(w, "          the disk looks empty; it will be erased anyway")
	}
	fmt.Fprintln(w, "          Other disks are not touched.")

	fmt.Fprintf(w, "\nLayout (GPT, UEFI):\n")
	tw := tabwriter.NewWriter(w, 0, 0, 2, ' ', 0)
	fmt.Fprintln(tw, "  #\tdevice\tlabel\tsize\tfs\tmount")
	for _, p := range in.Layout.Parts {
		mount := p.Mountpoint
		if p.Label == LabelRoot {
			mount = "/  (/var = subvolume " + VarSubvolume + ")"
		}
		size := binary(in.Layout.SizeOf(p))
		if p.Size == 0 {
			size = "~" + size + " (rest)"
		}
		fmt.Fprintf(tw, "  %d\t%s\t%s\t%s\t%s\t%s\n", p.Number, p.Device, p.Label, size, p.FS, mount)
	}
	tw.Flush()

	img := a.Image
	fmt.Fprintf(w, "\nImage:    %s\n", img.Source)
	fmt.Fprintf(w, "Updates:  %s (channel %s)\n", img.Target, img.Channel)
	if img.Enforce() {
		fmt.Fprintln(w, "          signature policy enforced (--enforce-container-sigpolicy)")
	} else {
		fmt.Fprintln(w, "          WARNING: signature policy NOT enforced (test images only)")
	}
	fmt.Fprintf(w, "Hostname: %s\n", a.Hostname)
	pw := "yescrypt hash from the answer file"
	if a.User.Password != "" {
		pw = "set from the answer file, stored as yescrypt"
	}
	fmt.Fprintf(w, "User:     %s (wheel), password %s", a.User.Name, pw)
	if n := len(a.User.SSHAuthorizedKeys); n > 0 {
		fmt.Fprintf(w, ", %d SSH key(s)", n)
	}
	fmt.Fprintln(w)
	if n := len(a.RootSSHAuthorizedKeys); n > 0 {
		fmt.Fprintf(w, "Root:     %d SSH key(s)\n", n)
	}
	switch {
	case a.Network.Mode == answers.NetworkStatic:
		fmt.Fprintf(w, "Network:  static %s via %s, DNS %s", a.Network.Address, a.Network.Gateway, strings.Join(a.Network.DNS, ", "))
	default:
		fmt.Fprint(w, "Network:  DHCP")
	}
	if a.Network.Interface != "" {
		fmt.Fprintf(w, " on %s", a.Network.Interface)
	} else {
		fmt.Fprint(w, " on any wired interface")
	}
	fmt.Fprintln(w)
	fmt.Fprintf(w, "Language: %s\n", a.Language)
	if len(a.KernelArgs) > 0 {
		fmt.Fprintf(w, "Kernel:   %s\n", strings.Join(a.KernelArgs, " "))
	}

	fmt.Fprintf(w, "\nCommands:\n")
	for _, c := range in.DiskCommands() {
		fmt.Fprintf(w, "  %s\n", indent(c.String()))
	}
	fmt.Fprintf(w, "  (mount %s at %s with /boot and /boot/efi)\n", in.Layout.Parts[2].Device, in.Root)
	fmt.Fprintf(w, "  %s\n", in.BootcCommand())
	fmt.Fprintf(w, "  (btrfs subvolume %s for /var, /etc/fstab, /etc/hostname, user, network, %s, SELinux labels)\n",
		VarSubvolume, InstallInfoPath)
	return nil
}

func indent(s string) string { return strings.ReplaceAll(s, "\n", "\n    ") }
