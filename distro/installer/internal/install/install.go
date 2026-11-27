// SPDX-License-Identifier: MPL-2.0

// Package install lays out the system disk and deploys Kinolune OS with
// `bootc install to-filesystem` (DIST-005, research 05 §2.4):
//
//  1. GPT, UEFI only: ESP 1 GiB (vfat), /boot 2 GiB (ext4), root btrfs
//     (the rest, or root_size_gib when a data partition is kept), optional
//     data partition (btrfs) on the rest;
//  2. `bootc install to-filesystem` from the offline payload in
//     containers-storage, tracking the registry (--target-imgref) and
//     enforcing the signature policy when the payload ships it;
//  3. /var moved into the btrfs subvolume @var (mounted from fstab);
//  4. hostname, the admin user (yescrypt, wheel), a NetworkManager
//     connection and /var/lib/alpha/install.json.
//
// Every write goes to the chosen disk or to file systems mounted from it.
package install

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
)

// DefaultRoot is where the target root file system is mounted.
const DefaultRoot = "/run/alpha-installer/target"

// Installer installs one system. Zero-value hooks get live defaults.
type Installer struct {
	Answers *answers.Answers
	Disk    disks.Disk
	Layout  Layout
	Runner  Runner
	// Root is the mount point of the target root (DefaultRoot).
	Root string
	// WorkDir holds temporary files (root SSH keys); default /run/alpha-installer.
	WorkDir string
	// Out receives progress lines.
	Out     io.Writer
	Version string

	// PasswordHash is the admin's yescrypt hash as written to the target's
	// /etc/shadow; set by Run. The TUI saves it in a repeat-install answer
	// file instead of the clear-text password.
	PasswordHash string

	// Progress receives the progress of every task (DIST-034); nil: none.
	Progress func(Event)
	// PayloadSize is the unpacked size of the image to install (bytes, 0:
	// unknown); it sizes the copy bar.
	PayloadSize func() int64
	// DiskWritten is the number of bytes written to the target disk so far;
	// nil: /sys/block/<disk>/stat.
	DiskWritten func() (int64, error)

	// Hooks for tests.
	Ticker     func(time.Duration) Ticker
	Chown      func(path string, uid, gid int) error
	Now        func() time.Time
	NewUUID    func() string
	WaitDevice func(ctx context.Context, path string) error

	progMu   sync.Mutex
	started  time.Time
	cur      StepID
	curDone  int64
	curTotal int64
	curNotes int
}

// New prepares an installer for disk d.
func New(a *answers.Answers, d disks.Disk, r Runner, out io.Writer) (*Installer, error) {
	l, err := PlanLayout(d.Path, d.Size, a.Disk)
	if err != nil {
		return nil, err
	}
	return &Installer{Answers: a, Disk: d, Layout: l, Runner: r, Out: out}, nil
}

func (in *Installer) defaults() {
	if in.Root == "" {
		in.Root = DefaultRoot
	}
	if in.WorkDir == "" {
		in.WorkDir = filepath.Dir(in.Root)
	}
	if in.Out == nil {
		in.Out = io.Discard
	}
	if in.Version == "" {
		in.Version = "dev"
	}
	if in.Chown == nil {
		in.Chown = os.Lchown
	}
	if in.Now == nil {
		in.Now = time.Now
	}
	if in.NewUUID == nil {
		in.NewUUID = randomUUID
	}
	if in.WaitDevice == nil {
		in.WaitDevice = waitDevice
	}
	if in.Ticker == nil {
		in.Ticker = newRealTicker
	}
}

func (in *Installer) step(format string, args ...any) {
	text := fmt.Sprintf(format, args...)
	fmt.Fprintf(in.Out, "==> %s\n", text)
	in.note(text)
}

func (in *Installer) run(ctx context.Context, name string, args ...string) ([]byte, error) {
	return in.Runner.Run(ctx, Cmd{Name: name, Args: args})
}

// Required lists the programs the installation needs on the live system.
func (in *Installer) Required() []string {
	req := []string{"wipefs", "sfdisk", "udevadm", "mkfs.vfat", "mkfs.ext4", "mkfs.btrfs", "btrfs",
		"mount", "umount", "blkid", "cp", "sync"}
	if in.Answers.Image.Runner == answers.RunnerPodman || in.Answers.Image.SignaturePolicyAuto() {
		req = append(req, "podman")
	} else {
		req = append(req, "bootc")
	}
	if in.Answers.User.Password != "" {
		req = append(req, "chpasswd")
	}
	return req
}

// Run performs the installation. It is destructive: the caller must have
// resolved the disk with ResolveTarget and obtained the user's confirmation.
func (in *Installer) Run(ctx context.Context) (err error) {
	in.defaults()
	in.started = in.Now()
	var missing []string
	for _, p := range in.Required() {
		if !in.Runner.LookPath(p) {
			missing = append(missing, p)
		}
	}
	if len(missing) > 0 {
		return fmt.Errorf("missing programs on the installer system: %s", strings.Join(missing, ", "))
	}
	defer func() {
		if err != nil {
			in.failCurrent(err)
		}
	}()

	// Before anything is written: a wrong answer here leaves a device that
	// can never update (BUG-031).
	if err := in.resolveSignaturePolicy(ctx); err != nil {
		return err
	}
	if err := in.partition(ctx); err != nil {
		return fmt.Errorf("partitioning %s: %w", in.Disk.Path, err)
	}
	if err := in.format(ctx); err != nil {
		return fmt.Errorf("creating file systems: %w", err)
	}
	if err := in.mount(ctx); err != nil {
		in.unmountAll(context.WithoutCancel(ctx))
		return fmt.Errorf("mounting the target: %w", err)
	}
	in.finish(StepFormat)
	mounted := true
	defer func() {
		if !mounted {
			return
		}
		if uerr := in.unmount(context.WithoutCancel(ctx)); uerr != nil {
			in.unmountAll(context.WithoutCancel(ctx))
		}
	}()
	if err := in.bootcInstall(ctx); err != nil {
		return fmt.Errorf("bootc install: %w", err)
	}
	in.begin(StepConfigure, in.configureUnits())
	if err := in.configure(ctx); err != nil {
		return fmt.Errorf("configuring the installed system: %w", err)
	}
	in.finish(StepConfigure)
	in.begin(StepFinalize, 4)
	mounted = false
	if err := in.unmount(context.WithoutCancel(ctx)); err != nil {
		in.unmountAll(context.WithoutCancel(ctx))
		return err
	}
	in.finish(StepFinalize)
	in.step("Installed Kinolune OS to %s; it tracks %s", in.Disk.Path, in.Answers.Image.Target)
	return nil
}

// DiskCommands are the commands that change the disk, in order, as the
// dry-run prints them.
func (in *Installer) DiskCommands() []Cmd {
	in.defaults()
	cmds := []Cmd{
		{Name: "wipefs", Args: []string{"--all", "--force", in.Disk.Path}},
		{Name: "sfdisk", Args: []string{"--wipe", "always", "--wipe-partitions", "always", in.Disk.Path},
			Stdin: in.Layout.SfdiskScript()},
		{Name: "udevadm", Args: []string{"settle", "--timeout=30"}},
	}
	for _, p := range in.Layout.Parts {
		cmds = append(cmds, mkfsCmd(p))
	}
	return cmds
}

func mkfsCmd(p Part) Cmd {
	switch p.FS {
	case "vfat":
		return Cmd{Name: "mkfs.vfat", Args: []string{"-F", "32", "-n", p.Label, p.Device}}
	case "ext4":
		return Cmd{Name: "mkfs.ext4", Args: []string{"-F", "-q", "-L", p.Label, p.Device}}
	default:
		return Cmd{Name: "mkfs.btrfs", Args: []string{"-f", "-q", "-L", p.Label, p.Device}}
	}
}

func (in *Installer) partition(ctx context.Context) error {
	in.step("Partitioning %s (%s, %s)", in.Disk.Path, in.Disk.Model, in.Disk.SizeString())
	cmds := in.DiskCommands()[:3]
	in.begin(StepPartition, int64(len(cmds)+len(in.Layout.Parts)))
	for _, c := range cmds {
		if _, err := in.Runner.Run(ctx, c); err != nil {
			return err
		}
		in.unitDone(StepPartition, c.Name)
	}
	for _, p := range in.Layout.Parts {
		if err := in.WaitDevice(ctx, p.Device); err != nil {
			return err
		}
		in.unitDone(StepPartition, p.Device)
	}
	in.finish(StepPartition)
	return nil
}

func (in *Installer) format(ctx context.Context) error {
	// One unit per file system and one for mounting them; done in Run.
	in.begin(StepFormat, int64(len(in.Layout.Parts)+1))
	for _, p := range in.Layout.Parts {
		in.step("Creating %s on %s (%s)", p.FS, p.Device, p.Label)
		// Old signatures inside the new partition (a previous install with the
		// same offsets) would confuse blkid; mkfs -f alone is not enough for vfat.
		if _, err := in.run(ctx, "wipefs", "--all", "--force", p.Device); err != nil {
			return err
		}
		if _, err := in.Runner.Run(ctx, mkfsCmd(p)); err != nil {
			return err
		}
		in.unitDone(StepFormat, p.FS+" "+p.Label)
	}
	return nil
}

func (in *Installer) mount(ctx context.Context) error {
	root, _ := in.Layout.Part(LabelRoot)
	boot, _ := in.Layout.Part(LabelBoot)
	esp, _ := in.Layout.Part(LabelESP)
	in.step("Mounting the target at %s", in.Root)
	if err := os.MkdirAll(in.Root, 0o755); err != nil {
		return err
	}
	if _, err := in.run(ctx, "mount", "-t", "btrfs", "-o", "compress=zstd:1", root.Device, in.Root); err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Join(in.Root, "boot"), 0o755); err != nil {
		return err
	}
	if _, err := in.run(ctx, "mount", "-t", "ext4", boot.Device, filepath.Join(in.Root, "boot")); err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Join(in.Root, "boot", "efi"), 0o755); err != nil {
		return err
	}
	_, err := in.run(ctx, "mount", "-t", "vfat", "-o", "umask=0077", esp.Device, filepath.Join(in.Root, "boot", "efi"))
	return err
}

func (in *Installer) unmount(ctx context.Context) error {
	in.step("Unmounting the target")
	var errs []error
	for _, m := range []string{filepath.Join(in.Root, "boot", "efi"), filepath.Join(in.Root, "boot"), in.Root} {
		if _, err := in.run(ctx, "umount", m); err != nil {
			errs = append(errs, err)
		}
		in.unitDone(StepFinalize, m)
	}
	if _, err := in.run(ctx, "sync"); err != nil {
		errs = append(errs, err)
	}
	in.unitDone(StepFinalize, "sync")
	return errors.Join(errs...)
}

// unmountAll is the best-effort cleanup after a failure.
func (in *Installer) unmountAll(ctx context.Context) {
	_, _ = in.run(ctx, "umount", "--recursive", "--lazy", in.Root)
}

// BootcCommand is the `bootc install to-filesystem` invocation.
func (in *Installer) BootcCommand() Cmd {
	in.defaults()
	img := in.Answers.Image
	args := []string{"install", "to-filesystem", "--target-imgref", img.Target}
	if img.Enforce() {
		args = append(args, "--enforce-container-sigpolicy")
	}
	// We write /etc and /var after bootc; finalize would remount read-only.
	args = append(args, "--skip-finalize")
	for _, k := range in.Answers.KernelArgs {
		args = append(args, "--karg", k)
	}
	keys := in.rootKeysFile()
	if keys != "" {
		args = append(args, "--root-ssh-authorized-keys", keys)
	}
	args = append(args, in.Root)

	if img.Runner == answers.RunnerHost {
		args = append([]string{args[0], args[1], "--source-imgref", img.Source}, args[2:]...)
		return Cmd{Name: "bootc", Args: args}
	}
	// podman: the payload image installs itself (05 §2.4). --pull=never: the
	// payload must come from the ISO, never from the network.
	pod := []string{"run", "--rm", "--privileged", "--pid=host", "--pull=never",
		"--security-opt", "label=type:unconfined_t",
		"-v", "/dev:/dev",
		"-v", "/var/lib/containers:/var/lib/containers",
		"-v", in.Root + ":" + in.Root}
	if keys != "" {
		pod = append(pod, "-v", keys+":"+keys+":ro")
	}
	pod = append(pod, img.SourceRef(), "bootc")
	return Cmd{Name: "podman", Args: append(pod, args...)}
}

func (in *Installer) rootKeysFile() string {
	if len(in.Answers.RootSSHAuthorizedKeys) == 0 {
		return ""
	}
	return filepath.Join(in.WorkDir, "root_authorized_keys")
}

func (in *Installer) bootcInstall(ctx context.Context) error {
	if f := in.rootKeysFile(); f != "" {
		data := strings.Join(in.Answers.RootSSHAuthorizedKeys, "\n") + "\n"
		if err := os.WriteFile(f, []byte(data), 0o600); err != nil {
			return err
		}
		defer os.Remove(f)
	}
	in.step("Installing %s (tracks %s)", in.Answers.Image.Source, in.Answers.Image.Target)
	m := in.startCopy()
	c := in.BootcCommand()
	c.Output = m
	_, err := in.Runner.Run(ctx, c)
	m.close(err == nil)
	return err
}

// PayloadVersionFile is the build record of the OS image (distro/Containerfile).
const PayloadVersionFile = "/usr/share/alpha/version"

// resolveSignaturePolicy decides --enforce-container-sigpolicy when the
// answer file leaves it unset. bootc stores it in the origin as the
// ostree-image-signed transport, and every later `bootc upgrade` then refuses
// to run under the stock policy (default insecureAcceptAnything). So enforce
// exactly when the payload ships our policy (ALPHA_SIGNATURE_POLICY=enforce,
// the project key is committed); an image built without the key gets the
// plain registry transport. Devices move to the signed transport later with
// `bootc switch --enforce-container-sigpolicy` (distro/README.md).
func (in *Installer) resolveSignaturePolicy(ctx context.Context) error {
	img := &in.Answers.Image
	if !img.SignaturePolicyAuto() {
		return nil
	}
	out, err := in.run(ctx, "podman", "run", "--rm", "--pull=never", "--entrypoint", "cat",
		img.SourceRef(), PayloadVersionFile)
	if err != nil {
		return fmt.Errorf("reading %s of the payload %s: %w", PayloadVersionFile, img.SourceRef(), err)
	}
	policy := ""
	for _, line := range strings.Split(string(out), "\n") {
		if v, ok := strings.CutPrefix(strings.TrimSpace(line), "ALPHA_SIGNATURE_POLICY="); ok {
			policy = v
		}
	}
	var enforce bool
	switch policy {
	case "enforce":
		enforce = true
	case "off":
		in.step("The payload has no signature policy yet (no project key): updates are not signature-checked")
	default:
		return fmt.Errorf("payload %s: unknown ALPHA_SIGNATURE_POLICY %q in %s; set image.enforce_signature_policy",
			img.SourceRef(), policy, PayloadVersionFile)
	}
	img.EnforceSignaturePolicy = &enforce
	return nil
}

// waitDevice waits until udev has created a partition node.
func waitDevice(ctx context.Context, path string) error {
	deadline := time.Now().Add(30 * time.Second)
	for {
		if _, err := os.Stat(path); err == nil {
			return nil
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("%s did not appear after partitioning", path)
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(200 * time.Millisecond):
		}
	}
}
