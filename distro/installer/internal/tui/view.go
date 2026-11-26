// SPDX-License-Identifier: MPL-2.0

package tui

import (
	"fmt"
	"regexp"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
	"github.com/DenisHumen/kinolune/distro/installer/internal/install"
)

// Styles use the 16 basic colours only (Linux console) and degrade to plain
// text on a serial console; the selection is always also marked with "> ".
var (
	stHeader = lipgloss.NewStyle().Reverse(true).Bold(true)
	stTitle  = lipgloss.NewStyle().Bold(true)
	stWarn   = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("1"))
	stOK     = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("2"))
	stSel    = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("6"))
	stMuted  = lipgloss.NewStyle().Foreground(lipgloss.Color("8"))
	stKeys   = lipgloss.NewStyle().Reverse(true)
)

// View implements tea.Model: header, body, message line and key hints,
// exactly m.h lines of at most m.w columns.
func (m *Model) View() string {
	cw := m.w - 2
	var body []string
	keys := ""
	switch {
	case m.quitOpen:
		body, keys = m.viewQuit(cw), tr(m.lang, "keys.menu")
	case m.scr == scrLang:
		body, keys = m.viewLang(cw), tr(m.lang, "keys.list")
	case m.scr == scrDisk:
		body, keys = m.viewDisk(cw), tr(m.lang, "keys.disk")
	case m.scr == scrLayout || m.scr == scrHost || m.scr == scrUser || m.scr == scrNet:
		body, keys = m.viewForm(cw), tr(m.lang, "keys.form")
	case m.scr == scrSummary:
		body, keys = m.viewSummary(cw), tr(m.lang, "keys.confirm")
	case m.scr == scrProgress:
		body, keys = m.viewProgress(cw), tr(m.lang, "keys.wait")
		if m.tasks != nil && !m.opt.DryRun {
			keys = tr(m.lang, "keys.tasks")
			if m.showLog {
				keys = tr(m.lang, "keys.log")
			}
		}
	case m.scr == scrDone:
		body, keys = m.viewDone(cw), tr(m.lang, "keys.menu")
	case m.scr == scrSave:
		body, keys = m.viewSave(cw), tr(m.lang, "keys.save")
	}

	left := " Kinolune OS " + m.opt.Version + " - " + tr(m.lang, "app")
	right := ""
	if m.scr <= scrSummary && !m.quitOpen {
		right = tr(m.lang, "step", int(m.scr)+1, wizardSteps) + " "
	}
	lines := []string{stHeader.Render(fit(left, right, m.w)), ""}
	room := m.h - 4
	if len(body) > room {
		body = body[:room]
	}
	for _, l := range body {
		lines = append(lines, " "+l)
	}
	for len(lines) < m.h-2 {
		lines = append(lines, "")
	}
	msgLine := ""
	if m.msg != "" {
		w := wrap(m.msg, cw)
		msgLine = " " + stWarn.Render(w[0])
	}
	lines = append(lines, msgLine, stKeys.Render(fit(" "+keys, "", m.w)))
	for i, l := range lines {
		lines[i] = truncate(l, m.w)
	}
	return strings.Join(lines, "\n")
}

// fit puts left and right on one line of width w, padded with spaces.
func fit(left, right string, w int) string {
	gap := w - lipgloss.Width(left) - lipgloss.Width(right)
	if gap < 1 {
		return truncate(left, w)
	}
	return left + strings.Repeat(" ", gap) + right
}

func truncate(s string, w int) string {
	if lipgloss.Width(s) <= w {
		return s
	}
	return lipgloss.NewStyle().MaxWidth(w).Render(s)
}

// wrap breaks s into lines of at most w columns at spaces.
func wrap(s string, w int) []string {
	var out []string
	for _, para := range strings.Split(s, "\n") {
		line := ""
		for _, word := range strings.Fields(para) {
			switch {
			case line == "":
				line = word
			case lipgloss.Width(line)+1+lipgloss.Width(word) <= w:
				line += " " + word
			default:
				out = append(out, line)
				line = word
			}
		}
		out = append(out, line)
	}
	return out
}

// hang wraps a numbered step with its continuation lines under the text.
func hang(s string, w int) []string {
	first := wrap(s, w)
	if len(first) == 1 {
		return first
	}
	out := first[:1]
	rest := strings.TrimSpace(strings.TrimPrefix(strings.TrimSpace(s), first[0]))
	for _, l := range wrap(rest, w-3) {
		out = append(out, "   "+l)
	}
	return out
}

func pad(s string, w int) string {
	return s + strings.Repeat(" ", max(0, w-lipgloss.Width(s)))
}

var ansiRe = regexp.MustCompile(`\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*\x07|\x1b[@-Z\\-_]`)

func stripANSI(s string) string { return ansiRe.ReplaceAllString(s, "") }

func marker(sel bool) string {
	if sel {
		return "> "
	}
	return "  "
}

func serial(d disks.Disk) string {
	s := strings.ReplaceAll(d.MaskedSerial(), "…", "...")
	if s == "" {
		return ""
	}
	return "S/N " + s
}

// --- screens -------------------------------------------------------------

func (m *Model) viewLang(cw int) []string {
	out := []string{stTitle.Render(tr(m.langIdx, "lang.title")), ""}
	for i, l := range Languages {
		row := marker(i == m.langIdx) + l.Name
		if i == m.langIdx {
			row = stSel.Render(row)
		}
		out = append(out, row)
	}
	out = append(out, "")
	out = append(out, wrap(tr(m.langIdx, "lang.intro"), cw)...)
	if m.opt.DryRun {
		out = append(out, "", stOK.Render(tr(m.langIdx, "sum.dryrun")))
	}
	return out
}

func (m *Model) viewDisk(cw int) []string {
	out := []string{stTitle.Render(tr(m.lang, "disk.title"))}
	out = append(out, wrap(tr(m.lang, "disk.intro"), cw)...)
	out = append(out, "")
	if m.diskErr != nil {
		out = append(out, stWarn.Render(tr(m.lang, "disk.error", m.diskErr.Error())))
	}
	if len(m.cands) == 0 {
		out = append(out, wrap(tr(m.lang, "disk.none", disks.FormatDecimal(disks.DefaultMinSize)), cw)...)
	}
	var wModel, wSize, wKind, wPath int
	for _, d := range m.cands {
		wModel = max(wModel, min(lipgloss.Width(d.Model), 28))
		wSize = max(wSize, lipgloss.Width(d.SizeString()))
		wKind = max(wKind, lipgloss.Width(d.KindLabel()))
		wPath = max(wPath, lipgloss.Width(d.Path))
	}
	// At most 6 rows, scrolled around the selection.
	first := 0
	if m.diskIdx >= 6 {
		first = m.diskIdx - 5
	}
	for i := first; i < len(m.cands) && i < first+6; i++ {
		d := m.cands[i]
		mark := " "
		if d.HasData() {
			mark = "!"
		}
		row := marker(i == m.diskIdx) + mark + " " + pad(truncate(d.Model, 28), wModel) + "  " +
			strings.Repeat(" ", wSize-lipgloss.Width(d.SizeString())) + d.SizeString() + "  " +
			pad(d.KindLabel(), wKind) + "  " + pad(d.Path, wPath)
		if s := serial(d); s != "" {
			row += "  " + s
		}
		row = strings.TrimRight(row, " ")
		if i == m.diskIdx {
			row = stSel.Render(row)
		}
		out = append(out, row)
	}
	if len(m.cands) > 0 {
		d := m.cands[m.diskIdx]
		out = append(out, "")
		if d.HasData() || len(d.Findings) > 0 {
			out = append(out, stWarn.Render(tr(m.lang, "disk.erase")+" "+d.Path))
			out = append(out, m.findingLines(d, 3)...)
		} else {
			out = append(out, tr(m.lang, "disk.empty")+": "+d.Path)
		}
	}
	if len(m.blocked) > 0 {
		out = append(out, "", tr(m.lang, "disk.excluded"))
		virtual := 0
		for _, b := range m.blocked {
			if b.disk.ExcludeReason == disks.ExcludeVirtual {
				virtual++
				continue
			}
			out = append(out, stMuted.Render("  - "+b.disk.Model+" "+b.disk.SizeString()+" "+b.disk.Path+": "+b.reason))
		}
		if virtual > 0 {
			out = append(out, stMuted.Render("  - "+tr(m.lang, "disk.virtual", virtual)))
		}
	}
	return out
}

func (m *Model) findingLines(d disks.Disk, limit int) []string {
	var out []string
	for i, f := range d.Findings {
		if i == limit {
			out = append(out, "    "+tr(m.lang, "disk.more", len(d.Findings)-limit))
			break
		}
		out = append(out, "  - "+findingText(m.lang, f))
	}
	return out
}

var formTitles = map[screen][2]string{
	scrLayout: {"layout.title", "layout.intro"},
	scrHost:   {"host.title", "host.intro"},
	scrUser:   {"user.title", "user.intro"},
	scrNet:    {"net.title", "net.intro"},
}

func (m *Model) viewForm(cw int) []string {
	t := formTitles[m.scr]
	out := []string{stTitle.Render(tr(m.lang, t[0]))}
	out = append(out, wrap(tr(m.lang, t[1]), cw)...)
	out = append(out, "")
	f := m.forms[m.scr]
	labelW := 0
	vis := m.visible(f)
	for _, x := range vis {
		if x.kind != fToggle {
			labelW = max(labelW, lipgloss.Width(tr(m.lang, x.label)))
		}
	}
	for i, x := range vis {
		sel := i == f.focus
		var row string
		switch x.kind {
		case fToggle:
			box := "[ ] "
			if x.on {
				box = "[x] "
			}
			row = marker(sel) + box + tr(m.lang, x.label)
		case fChoice:
			row = marker(sel) + pad(tr(m.lang, x.label), labelW) + "  < " + x.opts[x.sel] + " >"
		default:
			row = marker(sel) + pad(tr(m.lang, x.label), labelW) + "  [" + pad(x.in.View(), x.in.Width) + "]"
		}
		if sel {
			row = stSel.Render(row)
		}
		out = append(out, row)
	}
	out = append(out, "")
	switch m.scr {
	case scrLayout:
		out = append(out, m.layoutText())
		out = append(out, stMuted.Render(tr(m.lang, "layout.luks")))
	case scrUser:
		if m.base().User.PasswordHash != "" {
			out = append(out, wrap(tr(m.lang, "user.keep"), cw)...)
		}
	}
	return out
}

func (m *Model) layoutText() string {
	if m.target == nil {
		return ""
	}
	a := m.buildAnswers()
	if !a.Disk.DataPartition {
		return tr(m.lang, "layout.whole", m.target.SizeString())
	}
	rest := uint64(0)
	if used := install.ESPSize + install.BootSize + uint64(max(a.Disk.RootSizeGiB, 0))*install.GiB; m.target.Size > used {
		rest = m.target.Size - used
	}
	return tr(m.lang, "layout.split", a.Disk.RootSizeGiB, disks.FormatDecimal(rest), a.Disk.DataPool)
}

func (m *Model) netText(a *answers.Answers) string {
	n := a.Network
	s := tr(m.lang, "net.dhcp")
	if n.Mode == answers.NetworkStatic {
		s = tr(m.lang, "sum.static", n.Address, n.Gateway, strings.Join(n.DNS, ", "))
	}
	iface := tr(m.lang, "net.any")
	if n.Interface != "" {
		iface = tr(m.lang, "sum.on", n.Interface)
	}
	return s + "; " + iface
}

func (m *Model) viewSummary(cw int) []string {
	a, d := m.final, m.target
	rows := [][2]string{
		{"sum.disk", strings.TrimSuffix(d.Model+", "+d.SizeString()+", "+d.Path+", "+serial(*d), ", ")},
		{"sum.layout", m.layoutText()},
		{"sum.host", a.Hostname},
		{"sum.user", strings.TrimSpace(a.User.Name + " " + paren(a.User.FullName))},
		{"sum.net", m.netText(a)},
		{"sum.updates", tr(m.lang, "sum.channel", a.Image.Channel, a.Image.Target)},
		{"sum.lang", Languages[m.lang].Name},
	}
	labelW := 0
	for _, r := range rows {
		labelW = max(labelW, lipgloss.Width(tr(m.lang, r[0])))
	}
	out := []string{stTitle.Render(tr(m.lang, "sum.title")), ""}
	for _, r := range rows {
		for i, l := range wrap(r[1], cw-labelW-2) {
			label := ""
			if i == 0 {
				label = tr(m.lang, r[0])
			}
			out = append(out, pad(label, labelW)+"  "+l)
		}
	}
	out = append(out, "")
	if m.opt.DryRun {
		out = append(out, stOK.Render(tr(m.lang, "sum.dryrun")))
	} else {
		for _, l := range wrap(tr(m.lang, "sum.wipe", d.Path), cw) {
			out = append(out, stWarn.Render(l))
		}
		out = append(out, m.findingLines(*d, 2)...)
	}
	out = append(out, "", tr(m.lang, "sum.confirm", d.Name))
	out = append(out, stSel.Render("> ["+pad(m.confirm.View(), m.confirm.Width)+"]"))
	return out
}

func paren(s string) string {
	if s == "" {
		return ""
	}
	return "(" + s + ")"
}

func (m *Model) elapsed() string {
	return m.now.Sub(m.started).Round(time.Second).String()
}

func (m *Model) viewProgress(cw int) []string {
	title := tr(m.lang, "prog.title")
	if m.opt.DryRun {
		title = tr(m.lang, "prog.dry")
	}
	out := []string{stTitle.Render(title), ""}
	if m.tasks != nil && !m.opt.DryRun && !m.showLog {
		return append(out, m.viewTasks(cw)...)
	}
	step := m.stepLine
	if step == "" {
		step = "..."
	}
	out = append(out, stSel.Render(truncate(step, cw)), tr(m.lang, "prog.elapsed", m.elapsed()), "")
	room := m.h - 4 - len(out) - 2
	out = append(out, m.tail(room, cw)...)
	if m.opt.LogPath != "" {
		for len(out) < m.h-4-1 {
			out = append(out, "")
		}
		out = append(out, stMuted.Render(tr(m.lang, "prog.log", m.opt.LogPath)))
	}
	return out
}

// Column widths of the task list (DIST-034): it fits the 80x24 console.
const (
	taskName = 26
	taskBar  = 22
)

// nowMs is the screen time in milliseconds since the installation started.
func (m *Model) nowMs() int64 { return m.now.Sub(m.started).Milliseconds() }

// viewTasks draws one line per task with its bar, like pacman, and the
// total bar with the time left on top.
func (m *Model) viewTasks(cw int) []string {
	now := max(m.nowMs(), m.tasks.Last)
	name := min(taskName, max(cw-taskBar-14, 10))
	total := m.tasks.Overall(now)
	line := "     " + pad(tr(m.lang, "prog.total"), name) + " " + bar(total, taskBar, now) + " " + percent(total)
	if m.tasks.Failed() == nil {
		line += "  " + tr(m.lang, "prog.left", dur(m.tasks.Remaining(now)))
	}
	out := []string{stTitle.Render(truncate(line, cw)), "     " + tr(m.lang, "prog.elapsed", m.elapsed()), ""}
	for _, k := range m.tasks.Tasks {
		mark, st := "[  ]", stMuted
		switch k.State {
		case install.Running:
			mark, st = "[>>]", stSel
		case install.Finished:
			mark, st = "[ok]", stOK
		case install.Failed:
			mark, st = "[!!]", stWarn
		}
		row := pad(tr(m.lang, "task."+string(k.ID)), name)
		if k.State != install.Pending {
			f := k.Fraction()
			row += " " + bar(f, taskBar, now) + " " + percent(f) + " " + fmt.Sprintf("%6s", dur(k.Elapsed(now)))
		}
		out = append(out, truncate(st.Render(mark)+" "+row, cw))
		if detail := m.taskDetail(k); detail != "" {
			out = append(out, stMuted.Render(truncate("     "+detail, cw)))
		}
	}
	if m.opt.LogPath != "" {
		// Below the title and its blank line (viewProgress).
		for len(out) < m.h-4-1-2 {
			out = append(out, "")
		}
		out = append(out, stMuted.Render(truncate(tr(m.lang, "prog.log", m.opt.LogPath), cw)))
	}
	return out
}

// taskDetail is the second line of the running or failed task: amounts,
// speed and time left of the copy, the error of a failed task.
func (m *Model) taskDetail(k *install.Task) string {
	switch {
	case k.State == install.Failed:
		return k.Error
	case k.State != install.Running || !k.Bytes:
		return ""
	case k.Total > 0 && k.Speed > 0:
		eta, _ := k.ETA()
		return tr(m.lang, "prog.copy", size(k.Done), size(k.Total), speed(k.Speed), dur(eta))
	case k.Done > 0:
		return tr(m.lang, "prog.written", size(k.Done), speed(k.Speed))
	}
	return ""
}

// bar draws [#####-----]; an unknown amount (f < 0) is a block that runs
// back and forth.
func bar(f float64, w int, now int64) string {
	in := w - 2
	if f < 0 {
		const blk = 4
		span := in - blk
		pos := int(now/250) % (2 * span)
		if pos > span {
			pos = 2*span - pos
		}
		return "[" + strings.Repeat("-", pos) + strings.Repeat("#", blk) + strings.Repeat("-", span-pos) + "]"
	}
	n := int(f*float64(in) + 0.5)
	return "[" + strings.Repeat("#", n) + strings.Repeat("-", in-n) + "]"
}

func percent(f float64) string {
	if f < 0 {
		return "    "
	}
	return fmt.Sprintf("%3d%%", int(f*100))
}

// dur rounds to seconds: 3m05s.
func dur(d time.Duration) string {
	return d.Round(time.Second).String()
}

// speed is a write speed: 8.5 MB, 120 MB (the "/s" is in the text).
func speed(v float64) string {
	if v >= 1e6 && v < 100e6 {
		return fmt.Sprintf("%.1f MB", v/1e6)
	}
	return size(int64(v))
}

// size is a decimal size for amounts and speeds: 612 MB, 1.4 GB.
func size(n int64) string {
	switch {
	case n >= 1e9:
		return fmt.Sprintf("%.1f GB", float64(n)/1e9)
	case n >= 1e6:
		return fmt.Sprintf("%d MB", n/1e6)
	case n >= 1e3:
		return fmt.Sprintf("%d kB", n/1e3)
	}
	return fmt.Sprintf("%d B", n)
}

func (m *Model) tail(n, cw int) []string {
	if n <= 0 {
		return nil
	}
	lines := m.logLines
	if len(lines) > n {
		lines = lines[len(lines)-n:]
	}
	out := make([]string, len(lines))
	for i, l := range lines {
		out[i] = stMuted.Render(truncate(l, cw))
	}
	return out
}

func (m *Model) menuLines() []string {
	var out []string
	for i, a := range m.menu {
		row := marker(i == m.menuIdx) + tr(m.lang, "act."+a)
		if i == m.menuIdx {
			row = stSel.Render(row)
		}
		out = append(out, row)
	}
	return out
}

// AdminURL is where the admin panel of the installed system answers.
func AdminURL(a *answers.Answers) string {
	return "https://" + a.Hostname + ".local"
}

func (m *Model) viewDone(cw int) []string {
	var out []string
	switch {
	case m.err != nil:
		out = append(out, stWarn.Render(tr(m.lang, "fail.title")), "")
		if m.tasks != nil {
			if k := m.tasks.Failed(); k != nil {
				out = append(out, tr(m.lang, "fail.task", tr(m.lang, "task."+string(k.ID))), "")
			}
		}
		for _, l := range wrap(m.err.Error(), cw) {
			out = append(out, l)
			if len(out) > 5 {
				break
			}
		}
		out = append(out, "", tr(m.lang, "fail.partial"))
		out = append(out, m.tail(5, cw)...)
		if m.opt.LogPath != "" {
			out = append(out, tr(m.lang, "prog.log", m.opt.LogPath))
		}
	case m.opt.DryRun:
		out = append(out, stOK.Render(tr(m.lang, "done.dry")), "", tr(m.lang, "done.dry1"), "")
		out = append(out, m.tail(m.h-4-len(out)-len(m.menu)-2, cw)...)
	default:
		a := m.final
		url := AdminURL(a)
		if a.Network.Mode == answers.NetworkStatic {
			url += " (https://" + strings.SplitN(a.Network.Address, "/", 2)[0] + ")"
		}
		out = append(out, stOK.Render(tr(m.lang, "done.title")), "", tr(m.lang, "done.next"))
		out = append(out, hang(tr(m.lang, "done.n1"), cw)...)
		out = append(out, hang(tr(m.lang, "done.n2", url), cw)...)
		for _, l := range wrap(tr(m.lang, "done.n2b"), cw-3) {
			out = append(out, "   "+l)
		}
		out = append(out, hang(tr(m.lang, "done.n3", a.User.Name), cw)...)
		out = append(out, "", tr(m.lang, "prog.elapsed", m.elapsed()))
	}
	out = append(out, "")
	return append(out, m.menuLines()...)
}

func (m *Model) viewSave(cw int) []string {
	out := []string{stTitle.Render(tr(m.lang, "save.title"))}
	out = append(out, wrap(tr(m.lang, "save.intro"), cw)...)
	if _, hashed := m.SavedAnswers(); hashed {
		out = append(out, tr(m.lang, "save.hash"))
	} else {
		for _, l := range wrap(tr(m.lang, "save.plain"), cw) {
			out = append(out, stWarn.Render(l))
		}
	}
	out = append(out, "")
	usb := 0
	for i, t := range m.saves {
		var row string
		if t.Device == "" {
			row = tr(m.lang, "save.local", t.Path)
		} else {
			usb++
			row = strings.Join(strings.Fields(fmt.Sprintf("%s %s %s %s (%s)", t.Device, quote(t.Label), t.FSType,
				disks.FormatDecimal(t.Size), t.Disk)), " ")
		}
		row = marker(i == m.saveIdx) + row
		if i == m.saveIdx {
			row = stSel.Render(row)
		}
		out = append(out, row)
	}
	if usb == 0 {
		out = append(out, "", tr(m.lang, "save.none"))
	}
	return out
}

func quote(s string) string {
	if s == "" {
		return ""
	}
	return fmt.Sprintf("%q", s)
}

func (m *Model) viewQuit(int) []string {
	out := []string{stTitle.Render(tr(m.lang, "quit.title")), ""}
	for i, a := range quitMenu {
		row := marker(i == m.quitIdx) + tr(m.lang, "act."+a)
		if i == m.quitIdx {
			row = stSel.Render(row)
		}
		out = append(out, row)
	}
	return out
}
