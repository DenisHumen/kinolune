# SPDX-License-Identifier: MPL-2.0
<#
.SYNOPSIS
  kq - live testing of the real Kinolune app on Windows (QA-017).

.DESCRIPTION
  Drives the installed release build (no test build, no emulation) from the
  outside, the way a person does: reads the screen through the accessibility
  tree Flutter exposes over MSAA, clicks with the real mouse, types with the
  real keyboard, takes screenshots, and watches the app's own diagnostics
  (logs, crash reports), memory and process.

  Every command appends to the run journal, so a session of commands is one
  reproducible run: runs\<time>\run.log, screenshots, trees, texts, findings.

  Usage: powershell -NoProfile -ExecutionPolicy Bypass -File kq.ps1 <command> [args]
  See tools/qa/windows/README.md for the procedure.

  Commands:
    start [-Fresh] [-Exe <path>] [-Kill]   launch the app, open a new run
    stop                                   close the app, restore the profile (-Fresh)
    restart                                close and start again in the same run (persistence)
    restore                                restore a profile left aside by -Fresh
    status                                 process, memory, new crashes, log errors
    tree [-All]                            what is on screen (-All: also off-screen)
    texts [<name>]                         visible texts, saved to texts\<name>.txt
    find <text> [-Role <role>]             elements whose name contains <text>
    click <text> [-Role <role>] [-Index n] [-Right] [-Double]   (scrolls to it if needed)
    clickxy <x> <y>                        click at window-relative pixels
    act <text> [-Role <role>] [-Index n]   MSAA default action (no mouse)
    type <text>                            type text into the focused field
    key <keys>                             e.g. Enter, Esc, Tab, Ctrl+F, Alt+Left, Space, F11
    scroll <down|up> [<n>] [-At <text>]    mouse wheel over the window or an element
    wait <text> [-Timeout <s>] [-Gone]     wait until a text appears (or is gone)
    shot [<name>]                          screenshot of the window
    explore                                visit every navigation section
    note <P0|P1|P2> <title> [-Steps ..] [-Expected ..] [-Actual ..]
    logs [-Tail <n>]                       copy diagnostics of this run, show the tail
    report                                 write report.md of the run
#>

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

# ---------------------------------------------------------------- arguments

$Command = if ($args.Count -gt 0) { [string]$args[0] } else { 'help' }
$Pos = New-Object System.Collections.Generic.List[string]
$Opt = @{}
for ($i = 1; $i -lt $args.Count; $i++) {
    $a = [string]$args[$i]
    if ($a -match '^-([A-Za-z]+)$') {
        $k = $Matches[1].ToLowerInvariant()
        if ($k -in @('fresh', 'kill', 'all', 'right', 'double', 'gone')) {
            $Opt[$k] = $true
        } elseif ($i + 1 -lt $args.Count) {
            $Opt[$k] = [string]$args[$i + 1]; $i++
        } else {
            throw "option -$k needs a value"
        }
    } else {
        $Pos.Add($a)
    }
}
function Opt($name, $default = $null) { if ($Opt.ContainsKey($name)) { $Opt[$name] } else { $default } }

# ---------------------------------------------------------------- paths

$QaRoot = if ($env:KQ_HOME) { $env:KQ_HOME } else { Join-Path $env:LOCALAPPDATA 'kinolune-qa' }
$RunsDir = Join-Path $QaRoot 'runs'
$CurrentFile = Join-Path $QaRoot 'current.json'
$DefaultExe = Join-Path $env:LOCALAPPDATA 'Programs\Kinolune\Kinolune.exe'
# Where the app keeps its data (path_provider on Windows: company "app.kinolune", product "Kinolune").
$ProfileRoaming = Join-Path $env:APPDATA 'app.kinolune'
$ProfileLocal = Join-Path $env:LOCALAPPDATA 'app.kinolune'
$DataDir = Join-Path $ProfileRoaming 'Kinolune'
$DiagDir = Join-Path $DataDir 'diagnostics'

New-Item -ItemType Directory -Force -Path $RunsDir | Out-Null

# ---------------------------------------------------------------- native helper

Add-Type -AssemblyName Accessibility
Add-Type -AssemblyName System.Drawing
if (-not ('KqNative' -as [type])) {
Add-Type -ReferencedAssemblies Accessibility, System.Drawing -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using Accessibility;

public class KqNode {
    public int Id; public int Depth; public int Role; public string Name; public string Value;
    public int X; public int Y; public int W; public int H; public int State; public IAccessible Acc; public int Child;
}

public static class KqNative {
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr l);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int size);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] static extern IntPtr SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("oleacc.dll")] static extern int AccessibleObjectFromWindow(IntPtr h, int id, ref Guid iid, [MarshalAs(UnmanagedType.IUnknown)] out object o);
    [DllImport("oleacc.dll")] static extern int AccessibleChildren(IAccessible p, int start, int n, [Out] object[] kids, out int got);

    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public int data; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct INPUTU { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public INPUTU u; }

    public static void DpiAware() { try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { } }

    static string Cls(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }

    /// The top-level window of process `pid` that hosts a Flutter view.
    public static IntPtr MainWindow(int pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h) && FlutterView(h) != IntPtr.Zero) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static IntPtr FlutterView(IntPtr top) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(top, (h, l) => { if (Cls(h) == "FLUTTERVIEW") { found = h; return false; } return true; }, IntPtr.Zero);
        return found;
    }

    public static int[] Rect(IntPtr h) { RECT r; GetWindowRect(h, out r); return new[] { r.L, r.T, r.R - r.L, r.B - r.T }; }

    public static void Front(IntPtr h) {
        if (IsIconic(h)) ShowWindow(h, 9);
        // Alt tap: Windows lets a process take the foreground after input.
        keybd_event(0x12, 0, 0, UIntPtr.Zero); keybd_event(0x12, 0, 2, UIntPtr.Zero);
        SetForegroundWindow(h);
    }

    // ------------------------------------------------------------ MSAA tree

    public static List<KqNode> Tree(IntPtr view) {
        var list = new List<KqNode>();
        var g = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71");
        object o;
        if (AccessibleObjectFromWindow(view, -4, ref g, out o) != 0 || o == null) return list;
        Walk((IAccessible)o, 0, list);
        return list;
    }

    static void Walk(IAccessible a, int depth, List<KqNode> list) {
        if (depth > 60) return;
        int n = 0; try { n = a.accChildCount; } catch { }
        if (n <= 0) return;
        var kids = new object[n]; int got = 0;
        try { AccessibleChildren(a, 0, n, kids, out got); } catch { return; }
        for (int i = 0; i < got; i++) {
            var c = kids[i] as IAccessible;
            if (c == null) continue;
            var node = new KqNode { Id = list.Count, Depth = depth, Acc = c, Child = 0 };
            try { node.Name = c.get_accName(0); } catch { }
            try { node.Value = c.get_accValue(0); } catch { }
            try { node.Role = Convert.ToInt32(c.get_accRole(0)); } catch { }
            try { node.State = Convert.ToInt32(c.get_accState(0)); } catch { }
            try { int x, y, w, h; c.accLocation(out x, out y, out w, out h, 0); node.X = x; node.Y = y; node.W = w; node.H = h; } catch { }
            list.Add(node);
            Walk(c, depth + 1, list);
        }
    }

    public static bool DefaultAction(KqNode n) { try { n.Acc.accDoDefaultAction(0); return true; } catch { return false; } }

    // ------------------------------------------------------------ input

    const uint MOVE = 0x0001, LDOWN = 0x0002, LUP = 0x0004, RDOWN = 0x0008, RUP = 0x0010, WHEEL = 0x0800;
    static INPUT Mouse(uint flags, int data) { var i = new INPUT { type = 0 }; i.u.mi.flags = flags; i.u.mi.data = data; return i; }
    static INPUT Key(ushort vk, ushort scan, uint flags) { var i = new INPUT { type = 1 }; i.u.ki.vk = vk; i.u.ki.scan = scan; i.u.ki.flags = flags; return i; }
    static void Send(params INPUT[] i) { SendInput((uint)i.Length, i, Marshal.SizeOf(typeof(INPUT))); }

    public static void Click(int x, int y, bool right, bool dbl) {
        SetCursorPos(x, y); System.Threading.Thread.Sleep(60);
        int times = dbl ? 2 : 1;
        for (int k = 0; k < times; k++) {
            if (right) Send(Mouse(RDOWN, 0), Mouse(RUP, 0)); else Send(Mouse(LDOWN, 0), Mouse(LUP, 0));
            System.Threading.Thread.Sleep(70);
        }
    }

    public static void Wheel(int x, int y, int notches) {
        SetCursorPos(x, y); System.Threading.Thread.Sleep(40);
        Send(Mouse(WHEEL, notches * 120));
    }

    public static void Type(string text) {
        foreach (char ch in text) { Send(Key(0, ch, 0x0004), Key(0, ch, 0x0004 | 0x0002)); System.Threading.Thread.Sleep(8); }
    }

    [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);

    /// Virtual key plus its scan code: Flutter on Windows reads the physical
    /// key (and so Ctrl, Shift of shortcuts) from the scan code.
    static INPUT VKey(ushort vk, bool up) {
        uint ext = ((vk >= 0x21 && vk <= 0x2E) || vk == 0x5B) ? 0x0001u : 0u;
        return Key(vk, (ushort)MapVirtualKey(vk, 0), ext | (up ? 0x0002u : 0u));
    }

    public static void Keys(ushort[] mods, ushort vk) {
        var list = new List<INPUT>();
        foreach (var m in mods) list.Add(VKey(m, false));
        list.Add(VKey(vk, false)); list.Add(VKey(vk, true));
        for (int i = mods.Length - 1; i >= 0; i--) list.Add(VKey(mods[i], true));
        Send(list.ToArray());
    }

    // ------------------------------------------------------------ screenshot

    /// Window content through PrintWindow (works under other windows); an
    /// all-black image falls back to a copy of the screen.
    public static void Shot(IntPtr h, string path) {
        var r = Rect(h);
        using (var bmp = new Bitmap(Math.Max(1, r[2]), Math.Max(1, r[3]))) {
            bool ok;
            using (var g = Graphics.FromImage(bmp)) { var dc = g.GetHdc(); ok = PrintWindow(h, dc, 2); g.ReleaseHdc(dc); }
            if (!ok || Blank(bmp)) {
                Front(h); System.Threading.Thread.Sleep(250);
                using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(r[0], r[1], 0, 0, bmp.Size);
            }
            bmp.Save(path, ImageFormat.Png);
        }
    }

    static bool Blank(Bitmap b) {
        for (int i = 1; i < 10; i++) {
            var c = b.GetPixel(b.Width * i / 10, b.Height * i / 10);
            if (c.R + c.G + c.B > 30) return false;
        }
        return true;
    }
}
'@
}
[KqNative]::DpiAware()

# ---------------------------------------------------------------- run state

$RoleNames = @{
    9 = 'window'; 10 = 'client'; 12 = 'menuitem'; 15 = 'document'; 20 = 'group'; 30 = 'link'; 33 = 'list'
    34 = 'listitem'; 37 = 'tab'; 40 = 'image'; 41 = 'text'; 42 = 'field'; 43 = 'button'; 44 = 'checkbox'
    45 = 'radio'; 46 = 'combobox'; 48 = 'progress'; 51 = 'slider'; 52 = 'spin'; 57 = 'menubutton'
}
function RoleName([int]$r) { if ($RoleNames.ContainsKey($r)) { $RoleNames[$r] } else { "role$r" } }

function Load-Run {
    if (-not (Test-Path $CurrentFile)) { throw "no run: start one with 'kq start'" }
    Get-Content -Raw -Encoding UTF8 $CurrentFile | ConvertFrom-Json
}
function Save-Run($run) { $run | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 $CurrentFile }

function Log($run, [string]$kind, [string]$text) {
    $line = '{0:yyyy-MM-ddTHH:mm:ss} {1,-6} {2}' -f (Get-Date), $kind, $text
    Add-Content -Encoding UTF8 -Path (Join-Path $run.dir 'run.log') -Value $line
    Write-Output $line
}

function App-Process($run) {
    if (-not $run.pid) { return $null }
    Get-Process -Id $run.pid -ErrorAction SilentlyContinue
}

function Window($run) {
    $p = App-Process $run
    if (-not $p) { throw "the app is not running (pid $($run.pid)): it closed or crashed; see 'kq status'" }
    $h = [KqNative]::MainWindow($p.Id)
    if ($h -eq [IntPtr]::Zero) { throw 'the app has no window yet' }
    $h
}

# New crash reports and error lines in the app's log since the last check:
# written to run.log as CRASH / LOGERR lines, so no failure goes unnoticed.
function Check-Health($run) {
    $crashes = Join-Path $DiagDir 'crashes'
    $seen = @($run.seenCrashes)
    if (Test-Path $crashes) {
        Get-ChildItem $crashes -File | Where-Object { $_.LastWriteTime -ge [datetime]$run.started -and $seen -notcontains $_.Name } |
            Sort-Object Name | ForEach-Object {
                $msg = (Get-Content -Encoding UTF8 $_.FullName | Select-String -Pattern 'panicked|Exception|Error' | Select-Object -First 2) -join ' / '
                Copy-Item $_.FullName (Join-Path $run.dir 'crashes') -Force
                Log $run 'CRASH' "$($_.Name): $msg"
                $seen += $_.Name
            }
    }
    $run.seenCrashes = $seen
    $log = Join-Path $DiagDir 'logs\kinolune.log'
    if (Test-Path $log) {
        $lines = Get-Content -Encoding UTF8 $log
        $from = [int]$run.logLines
        if ($lines.Count -lt $from) { $from = 0 }  # rotated
        for ($i = $from; $i -lt $lines.Count; $i++) {
            if ($lines[$i] -match ' (ERROR|WARN) ') { Log $run 'LOGERR' $lines[$i] }
        }
        $run.logLines = $lines.Count
    }
    $p = App-Process $run
    if (-not $p -and -not $run.stopped) { Log $run 'CRASH' "process $($run.pid) is gone" ; $run.stopped = $true }
    if ($p) {
        $mb = [math]::Round($p.PrivateMemorySize64 / 1MB)
        Add-Content -Encoding UTF8 -Path (Join-Path $run.dir 'memory.tsv') -Value ("{0:HH:mm:ss}`t{1}`t{2}`t{3}" -f (Get-Date), $mb, $p.Threads.Count, $p.HandleCount)
    }
    Save-Run $run
}

# ---------------------------------------------------------------- screen

function Read-Tree($run, [bool]$all) {
    $h = Window $run
    $view = [KqNative]::FlutterView($h)
    $r = [KqNative]::Rect($view)
    $nodes = [KqNative]::Tree($view)
    if ($nodes.Count -eq 0) {
        # The first request turns Flutter's semantics on; the tree comes a moment later.
        Start-Sleep -Milliseconds 800
        $nodes = [KqNative]::Tree($view)
    }
    $vis = foreach ($n in $nodes) {
        $on = $n.W -gt 0 -and $n.H -gt 0 -and $n.X + $n.W -gt $r[0] -and $n.X -lt $r[0] + $r[2] -and $n.Y + $n.H -gt $r[1] -and $n.Y -lt $r[1] + $r[3]
        if ($all -or $on) { $n }
    }
    @{ nodes = @($vis); view = $r; hwnd = $h }
}

function Node-Text($n, $view) {
    $name = ($n.Name -replace "`r?`n", ' / ').Trim()
    $flags = @()
    if ($n.State -band 0x10) { $flags += 'checked' }
    if ($n.State -band 0x1) { $flags += 'disabled' }
    if ($n.State -band 0x4) { $flags += 'focused' }
    if ($n.State -band 0x2) { $flags += 'selected' }
    $f = if ($flags) { ' {' + ($flags -join ',') + '}' } else { '' }
    $v = if ($n.Value) { " = '$($n.Value)'" } else { '' }
    '#{0,-4}{1}{2,-8} "{3}"{4}{5}  @{6},{7} {8}x{9}' -f $n.Id, ('  ' * [math]::Min($n.Depth, 12)), (RoleName $n.Role), $name, $v, $f,
        ($n.X - $view[0]), ($n.Y - $view[1]), $n.W, $n.H
}

function Find-Nodes($tree, [string]$text, [string]$role) {
    $c = @($tree.nodes | Where-Object { $_.Name -and $_.W -gt 0 })
    if ($role) { $c = @($c | Where-Object { (RoleName $_.Role) -eq $role }) }
    if ($text -match '^#(\d+)$') { return @($tree.nodes | Where-Object { $_.Id -eq [int]$Matches[1] }) }
    $norm = { param($s) (($s -replace "`r?`n", ' ') -replace '\s+', ' ').Trim() }
    $exact = @($c | Where-Object { (& $norm $_.Name) -ieq $text -or (($_.Name -split "`r?`n")[0].Trim() -ieq $text) })
    if ($exact.Count) { return $exact }
    @($c | Where-Object { (& $norm $_.Name).IndexOf($text, [StringComparison]::OrdinalIgnoreCase) -ge 0 })
}

# The visible tree with an element matching <text> on screen: an element
# that exists but is scrolled away is brought into view with the wheel.
function Tree-With($run, [string]$text) {
    $t = Read-Tree $run $false
    for ($i = 0; $i -lt 25; $i++) {
        if (@(Find-Nodes $t $text (Opt 'role')).Count) { return $t }
        $all = Read-Tree $run $true
        $m = @(Find-Nodes $all $text (Opt 'role'))
        if (-not $m.Count) { return $t }
        $target = $m[[math]::Min([int](Opt 'index' 1), $m.Count) - 1]
        $v = $all.view
        # Scroll the pane under the element's column: the left rail does not scroll.
        $x = [math]::Max($v[0] + 10, [math]::Min($target.X + 20, $v[0] + $v[2] - 10))
        $notches = if ($target.Y -lt $v[1]) { 3 } else { -3 }
        [KqNative]::Front($all.hwnd)
        [KqNative]::Wheel($x, $v[1] + [int]($v[3] / 2), $notches)
        Start-Sleep -Milliseconds 350
        $t = Read-Tree $run $false
    }
    $t
}

function Pick($run, $tree, [string]$text) {
    $found = Find-Nodes $tree $text (Opt 'role')
    if ($found.Count -eq 0) { throw "nothing on screen matches '$text' (try 'kq find' or 'kq tree')" }
    $idx = [int](Opt 'index' 1)
    if ($found.Count -gt 1 -and -not $Opt.ContainsKey('index')) {
        Log $run 'INFO' ("'$text' matches $($found.Count) elements, taking the first (use -Index n): " +
            (($found | Select-Object -First 5 | ForEach-Object { "#$($_.Id) $(RoleName $_.Role) '$(($_.Name -split "`r?`n")[0])'" }) -join '; '))
    }
    if ($idx -lt 1 -or $idx -gt $found.Count) { throw "-Index ${idx}: only $($found.Count) matches" }
    $found[$idx - 1]
}

function Shot($run, [string]$name) {
    $h = Window $run
    $n = [int]$run.shots + 1
    $run.shots = $n
    $safe = ($name -replace '[^\w\-]+', '-').Trim('-')
    $file = Join-Path $run.dir ('screenshots\{0:D3}-{1}.png' -f $n, $(if ($safe) { $safe } else { 'shot' }))
    [KqNative]::Shot($h, $file)
    Save-Run $run
    $file
}

function Visible-Texts($tree) {
    $seen = @{}
    foreach ($n in $tree.nodes) {
        if (-not $n.Name) { continue }
        $t = ($n.Name -replace "`r?`n", ' / ').Trim()
        if ($t -and -not $seen.ContainsKey($t)) { $seen[$t] = 1; '[{0}] {1}' -f (RoleName $n.Role), $t }
    }
}

# ---------------------------------------------------------------- commands

function Cmd-Start {
    $exe = Opt 'exe' $DefaultExe
    if (-not (Test-Path $exe)) { throw "no app at $exe (install a release or pass -Exe)" }
    $running = Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($exe)) -ErrorAction SilentlyContinue
    if ($running) {
        if (-not (Opt 'kill')) { throw "Kinolune is already running (pid $($running.Id -join ',')): close it or pass -Kill" }
        $running | ForEach-Object { $_.CloseMainWindow() | Out-Null }
        Start-Sleep -Seconds 3
        Get-Process -Id $running.Id -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Seconds 1
    }
    if (Test-Path $CurrentFile) {
        $old = Load-Run
        if ($old.fresh -and $old.backup -and -not $old.restored) {
            throw "the previous run $($old.id) still has the user's profile set aside: run 'kq restore' first"
        }
    }
    $id = Get-Date -Format 'yyyyMMdd-HHmmss'
    $dir = Join-Path $RunsDir $id
    foreach ($d in 'screenshots', 'trees', 'texts', 'crashes', 'logs') { New-Item -ItemType Directory -Force -Path (Join-Path $dir $d) | Out-Null }
    $version = (Get-Item $exe).VersionInfo.ProductVersion
    $run = [pscustomobject]@{
        id = $id; dir = $dir; exe = $exe; version = $version; started = (Get-Date).ToString('o'); pid = 0
        fresh = [bool](Opt 'fresh'); backup = $null; restored = $false; stopped = $false
        shots = 0; notes = 0; seenCrashes = @(); logLines = 0
        os = (Get-CimInstance Win32_OperatingSystem).Caption + ' ' + [Environment]::OSVersion.Version
    }
    if ($run.fresh) {
        # First run on a clean profile: the user's profile is moved aside (not
        # deleted) and comes back on 'kq stop' / 'kq restore'.
        $bk = Join-Path $QaRoot "profile-backup-$id"
        New-Item -ItemType Directory -Force -Path $bk | Out-Null
        if (Test-Path $ProfileRoaming) { Move-Item $ProfileRoaming (Join-Path $bk 'roaming') }
        if (Test-Path $ProfileLocal) { Move-Item $ProfileLocal (Join-Path $bk 'local') }
        $run.backup = $bk
    }
    # Diagnostics on from the first second (the app's own opt-in marker).
    New-Item -ItemType Directory -Force -Path $DiagDir | Out-Null
    if (-not (Test-Path (Join-Path $DiagDir 'opt-in'))) {
        Set-Content -Encoding ASCII -Path (Join-Path $DiagDir 'opt-in') -Value 'Diagnostics are on: enabled by tools/qa/windows/kq.ps1 for a live test run.'
        $run | Add-Member -NotePropertyName optInCreated -NotePropertyValue $true
    }
    $log = Join-Path $DiagDir 'logs\kinolune.log'
    if (Test-Path $log) { $run.logLines = (Get-Content -Encoding UTF8 $log).Count }
    $p = Start-Process -FilePath $exe -PassThru
    $run.pid = $p.Id
    Save-Run $run
    Log $run 'START' "run ${id}: $exe $version (pid $($p.Id))$(if ($run.fresh) { ', clean profile' })"
    Log $run 'INFO' "OS: $($run.os); screen scale and size: $((Get-CimInstance Win32_VideoController | Select-Object -First 1).CurrentHorizontalResolution)x$((Get-CimInstance Win32_VideoController | Select-Object -First 1).CurrentVerticalResolution)"
    $deadline = (Get-Date).AddSeconds(40)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 500
        if (-not (App-Process $run)) { break }
        if ([KqNative]::MainWindow($p.Id) -ne [IntPtr]::Zero) { break }
    }
    $sw = (Get-Date) - [datetime]$run.started
    if ([KqNative]::MainWindow($p.Id) -eq [IntPtr]::Zero) {
        Log $run 'FAIL' 'no window within 40 s'
    } else {
        Log $run 'STEP' ("window after {0:N1} s" -f $sw.TotalSeconds)
        Start-Sleep -Seconds 2
        $f = Shot $run 'start'
        Log $run 'SHOT' $f
    }
    Check-Health $run
}

function Cmd-Stop {
    $run = Load-Run
    Check-Health $run
    $p = App-Process $run
    if ($p) {
        $p.CloseMainWindow() | Out-Null
        if (-not $p.WaitForExit(8000)) { Stop-Process -Id $p.Id -Force; Log $run 'INFO' 'app did not close in 8 s: killed' }
        else { Log $run 'STEP' 'app closed' }
    }
    $run.stopped = $true
    Copy-Diagnostics $run
    if ($run.fresh -and $run.backup -and -not $run.restored) { Restore-Profile $run }
    elseif ($run.optInCreated) { Remove-Item -Force (Join-Path $DiagDir 'opt-in') -ErrorAction SilentlyContinue }
    Save-Run $run
    Write-Report $run
    Log $run 'STOP' "report: $(Join-Path $run.dir 'report.md')"
}

# Closes the app and starts it again in the same run (same profile):
# what was saved must still be there.
function Cmd-Restart {
    $run = Load-Run
    Check-Health $run
    $p = App-Process $run
    if ($p) {
        $p.CloseMainWindow() | Out-Null
        if (-not $p.WaitForExit(8000)) { Stop-Process -Id $p.Id -Force; Log $run 'INFO' 'app did not close in 8 s: killed' }
    }
    $p = Start-Process -FilePath $run.exe -PassThru
    $run.pid = $p.Id
    $run.stopped = $false
    Save-Run $run
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt 40 -and [KqNative]::MainWindow($p.Id) -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 400 }
    Log $run 'START' ("restarted (pid {0}), window after {1:N1} s" -f $p.Id, ((Get-Date) - $t0).TotalSeconds)
    Start-Sleep -Seconds 2
    $f = Shot $run 'restart'; Log $run 'SHOT' $f
    Check-Health $run
}

function Restore-Profile($run) {
    $test = Join-Path $run.dir 'profile'
    New-Item -ItemType Directory -Force -Path $test | Out-Null
    # Keep what the test profile wrote (database, settings, diagnostics) as evidence.
    if (Test-Path $ProfileRoaming) { Move-Item $ProfileRoaming (Join-Path $test 'roaming') }
    if (Test-Path $ProfileLocal) { Remove-Item -Recurse -Force $ProfileLocal }  # caches only
    $bk = $run.backup
    if (Test-Path (Join-Path $bk 'roaming')) { Move-Item (Join-Path $bk 'roaming') $ProfileRoaming }
    if (Test-Path (Join-Path $bk 'local')) { Move-Item (Join-Path $bk 'local') $ProfileLocal }
    Remove-Item -Force -Recurse $bk -ErrorAction SilentlyContinue
    $run.restored = $true
    Save-Run $run
    Log $run 'INFO' 'the user profile is back in place'
}

function Cmd-Restore {
    $run = Load-Run
    if (-not ($run.fresh -and $run.backup -and -not $run.restored)) { Write-Output 'nothing to restore'; return }
    $p = App-Process $run
    if ($p) { Stop-Process -Id $p.Id -Force; Start-Sleep -Seconds 1 }
    Restore-Profile $run
}

function Copy-Diagnostics($run) {
    $log = Join-Path $DiagDir 'logs\kinolune.log'
    if (Test-Path $log) { Copy-Item $log (Join-Path $run.dir 'logs\kinolune.log') -Force }
}

function Cmd-Status {
    $run = Load-Run
    Check-Health $run
    $p = App-Process $run
    if ($p) {
        $h = [KqNative]::MainWindow($p.Id)
        $r = if ($h -ne [IntPtr]::Zero) { ([KqNative]::Rect($h)) -join ',' } else { 'no window' }
        Write-Output ("running: pid {0}, private {1} MB, working set {2} MB, threads {3}, handles {4}, window {5}, cpu {6:N0} s" -f
            $p.Id, [math]::Round($p.PrivateMemorySize64 / 1MB), [math]::Round($p.WorkingSet64 / 1MB), $p.Threads.Count, $p.HandleCount, $r, $p.TotalProcessorTime.TotalSeconds)
    } else { Write-Output 'not running' }
    Write-Output "run: $($run.id)  dir: $($run.dir)"
    $crashes = @(Get-Content -Encoding UTF8 (Join-Path $run.dir 'run.log') | Select-String ' CRASH ').Count
    $errs = @(Get-Content -Encoding UTF8 (Join-Path $run.dir 'run.log') | Select-String ' LOGERR ').Count
    Write-Output "crashes: $crashes, log errors/warnings: $errs, findings: $($run.notes)"
}

function Cmd-Tree {
    $run = Load-Run
    $t = Read-Tree $run ([bool](Opt 'all'))
    $lines = $t.nodes | ForEach-Object { Node-Text $_ $t.view }
    $file = Join-Path $run.dir ('trees\{0:HHmmss}.txt' -f (Get-Date))
    $lines | Set-Content -Encoding UTF8 $file
    $lines
    Write-Output "-- $($t.nodes.Count) elements, window $($t.view -join ','), saved $file"
    Check-Health $run
}

function Cmd-Texts {
    $run = Load-Run
    $t = Read-Tree $run $false
    $name = if ($Pos.Count) { $Pos[0] } else { Get-Date -Format 'HHmmss' }
    $lines = @(Visible-Texts $t)
    $file = Join-Path $run.dir "texts\$(($name -replace '[^\w\-]+','-'))-$(Get-Date -Format HHmmss).txt"
    $lines | Set-Content -Encoding UTF8 $file
    $lines
    Write-Output "-- saved $file"
}

function Cmd-Find {
    $run = Load-Run
    if (-not $Pos.Count) { throw 'find <text>' }
    $t = Read-Tree $run ([bool](Opt 'all'))
    Find-Nodes $t $Pos[0] (Opt 'role') | ForEach-Object { Node-Text $_ $t.view }
}

function Cmd-Click {
    $run = Load-Run
    if (-not $Pos.Count) { throw 'click <text>' }
    $t = Tree-With $run $Pos[0]
    $n = Pick $run $t $Pos[0]
    [KqNative]::Front($t.hwnd)
    $x = [int]($n.X + $n.W / 2); $y = [int]($n.Y + $n.H / 2)
    [KqNative]::Click($x, $y, [bool](Opt 'right'), [bool](Opt 'double'))
    Log $run 'CLICK' ("'{0}' -> #{1} {2} '{3}' @{4},{5}" -f $Pos[0], $n.Id, (RoleName $n.Role), (($n.Name -split "`r?`n")[0]), ($x - $t.view[0]), ($y - $t.view[1]))
    Start-Sleep -Milliseconds ([int](Opt 'settle' 900))
    Check-Health $run
}

function Cmd-ClickXY {
    $run = Load-Run
    if ($Pos.Count -lt 2) { throw 'clickxy <x> <y> (window-relative)' }
    $h = Window $run
    $v = [KqNative]::Rect([KqNative]::FlutterView($h))
    [KqNative]::Front($h)
    [KqNative]::Click($v[0] + [int]$Pos[0], $v[1] + [int]$Pos[1], [bool](Opt 'right'), [bool](Opt 'double'))
    Log $run 'CLICK' "@$($Pos[0]),$($Pos[1])"
    Start-Sleep -Milliseconds 900
    Check-Health $run
}

function Cmd-Act {
    $run = Load-Run
    if (-not $Pos.Count) { throw 'act <text>' }
    $t = Tree-With $run $Pos[0]
    $n = Pick $run $t $Pos[0]
    $ok = [KqNative]::DefaultAction($n)
    Log $run 'ACT' ("'{0}' -> #{1} '{2}': {3}" -f $Pos[0], $n.Id, (($n.Name -split "`r?`n")[0]), $(if ($ok) { 'done' } else { 'no default action' }))
    Start-Sleep -Milliseconds 900
    Check-Health $run
}

function Cmd-Type {
    $run = Load-Run
    $text = $Pos -join ' '
    [KqNative]::Front((Window $run))
    [KqNative]::Type($text)
    Log $run 'TYPE' $text
    Start-Sleep -Milliseconds 500
    Check-Health $run
}

$Vk = @{
    enter = 0x0D; esc = 0x1B; escape = 0x1B; tab = 0x09; space = 0x20; back = 0x08; backspace = 0x08; delete = 0x2E; del = 0x2E
    left = 0x25; up = 0x26; right = 0x27; down = 0x28; home = 0x24; end = 0x23; pageup = 0x21; pagedown = 0x22
    f1 = 0x70; f2 = 0x71; f3 = 0x72; f4 = 0x73; f5 = 0x74; f6 = 0x75; f7 = 0x76; f8 = 0x77; f9 = 0x78; f10 = 0x79; f11 = 0x7A; f12 = 0x7B
    mediaplay = 0xB3; volumeup = 0xAF; volumedown = 0xAE; volumemute = 0xAD
}
$Mods = @{ ctrl = 0x11; control = 0x11; shift = 0x10; alt = 0x12; win = 0x5B }

function Cmd-Key {
    $run = Load-Run
    if (-not $Pos.Count) { throw 'key <keys>, e.g. Enter, Ctrl+F, Alt+Left' }
    [KqNative]::Front((Window $run))
    foreach ($combo in $Pos) {
        $parts = $combo.ToLowerInvariant() -split '\+'
        $mods = [uint16[]]@($parts | Select-Object -SkipLast 1 | ForEach-Object { if (-not $Mods.ContainsKey($_)) { throw "unknown modifier $_" }; [uint16]$Mods[$_] })
        $k = $parts[-1]
        $code = if ($Vk.ContainsKey($k)) { $Vk[$k] } elseif ($k.Length -eq 1) { [int][char]$k.ToUpperInvariant() } else { throw "unknown key $k" }
        [KqNative]::Keys($mods, [uint16]$code)
        Log $run 'KEY' $combo
        Start-Sleep -Milliseconds 300
    }
    Start-Sleep -Milliseconds 500
    Check-Health $run
}

function Cmd-Scroll {
    $run = Load-Run
    $dir = if ($Pos.Count) { $Pos[0] } else { 'down' }
    $n = if ($Pos.Count -gt 1) { [int]$Pos[1] } else { 5 }
    $t = Read-Tree $run $false
    if (Opt 'at') { $e = Pick $run $t (Opt 'at'); $x = $e.X + [int]($e.W / 2); $y = $e.Y + [int]($e.H / 2) }
    else { $x = $t.view[0] + [int]($t.view[2] * 0.6); $y = $t.view[1] + [int]($t.view[3] / 2) }
    [KqNative]::Front($t.hwnd)
    [KqNative]::Wheel($x, $y, $(if ($dir -eq 'up') { $n } else { -$n }))
    Log $run 'SCROLL' "$dir $n"
    Start-Sleep -Milliseconds 700
    Check-Health $run
}

function Cmd-Wait {
    $run = Load-Run
    if (-not $Pos.Count) { throw 'wait <text>' }
    $timeout = [int](Opt 'timeout' 20)
    $gone = [bool](Opt 'gone')
    $start = Get-Date
    while ($true) {
        $t = Read-Tree $run $true
        $hit = @(Find-Nodes $t $Pos[0] (Opt 'role')).Count -gt 0
        if ($hit -ne $gone) {
            Log $run 'WAIT' ("'{0}' {1} after {2:N1} s" -f $Pos[0], $(if ($gone) { 'gone' } else { 'shown' }), ((Get-Date) - $start).TotalSeconds)
            break
        }
        if (((Get-Date) - $start).TotalSeconds -gt $timeout) {
            Log $run 'FAIL' ("'{0}' not {1} in {2} s" -f $Pos[0], $(if ($gone) { 'gone' } else { 'shown' }), $timeout)
            $f = Shot $run 'wait-timeout'; Log $run 'SHOT' $f
            Check-Health $run
            exit 1
        }
        Start-Sleep -Milliseconds 700
    }
    Check-Health $run
}

function Cmd-Shot {
    $run = Load-Run
    $f = Shot $run $(if ($Pos.Count) { $Pos[0] } else { '' })
    Log $run 'SHOT' $f
    Check-Health $run
}

# Visits every navigation section (buttons of the left rail): screenshot,
# visible texts and a health check after each. A breadth pass before the
# deep, hand-driven one.
function Cmd-Explore {
    $run = Load-Run
    $t = Read-Tree $run $false
    $left = $t.view[0]
    # The rail: the largest column of buttons with the same left edge and
    # width near the left side (a hero "Play" button is near it too).
    $cands = @($t.nodes | Where-Object { $_.Role -eq 43 -and $_.Name -and $_.X -lt $left + 320 -and $_.W -lt 320 -and $_.H -lt 80 })
    $rail = $cands | Group-Object { "$($_.X)/$($_.W)" } | Sort-Object Count -Descending | Select-Object -First 1
    $nav = @($rail.Group | ForEach-Object { ($_.Name -split "`r?`n")[0].Trim() } | Select-Object -Unique)
    if (-not $nav.Count) { throw 'no navigation rail found on this screen (onboarding? use click/tree)' }
    Log $run 'STEP' "explore: $($nav -join ', ')"
    foreach ($name in $nav) {
        $tt = Read-Tree $run $false
        $n = @(Find-Nodes $tt $name 'button' | Where-Object { $_.X -lt $left + 320 })[0]
        if (-not $n) { Log $run 'FAIL' "explore: '$name' is no longer on screen"; continue }
        [KqNative]::Front($tt.hwnd)
        [KqNative]::Click([int]($n.X + $n.W / 2), [int]($n.Y + $n.H / 2), $false, $false)
        Start-Sleep -Seconds 2
        $after = Read-Tree $run $false
        $texts = @(Visible-Texts $after)
        $safe = ($name -replace '[^\w\-]+', '-')
        $texts | Set-Content -Encoding UTF8 (Join-Path $run.dir "texts\section-$safe.txt")
        $f = Shot $run "section-$safe"
        Log $run 'STEP' ("section '{0}': {1} elements, {2} texts, {3}" -f $name, $after.nodes.Count, $texts.Count, (Split-Path -Leaf $f))
        Check-Health $run
        if (-not (App-Process $run)) { break }
    }
}

function Cmd-Note {
    $run = Load-Run
    if ($Pos.Count -lt 2 -or $Pos[0] -notmatch '^P[012]$') { throw 'note <P0|P1|P2> <title> [-Steps ..] [-Expected ..] [-Actual ..]' }
    $n = [int]$run.notes + 1
    $run.notes = $n
    $shot = Shot $run "finding-$n"
    $file = Join-Path $run.dir 'findings.md'
    if (-not (Test-Path $file)) {
        Set-Content -Encoding UTF8 $file "# Находки прогона $($run.id)`n`nKinolune $($run.version), $($run.os)`n"
    }
    $title = ($Pos | Select-Object -Skip 1) -join ' '
    $body = @("## F$n [$($Pos[0])] $title", '')
    if (Opt 'steps') { $body += "- Шаги: $(Opt 'steps')" }
    if (Opt 'expected') { $body += "- Ожидалось: $(Opt 'expected')" }
    if (Opt 'actual') { $body += "- Получилось: $(Opt 'actual')" }
    $body += "- Скриншот: screenshots/$(Split-Path -Leaf $shot)"
    $body += "- Время: $(Get-Date -Format 'yyyy-MM-ddTHH:mm:ss')"
    $body += ''
    Add-Content -Encoding UTF8 $file ($body -join "`n")
    Save-Run $run
    Log $run 'NOTE' "F$n [$($Pos[0])] $title"
}

function Cmd-Logs {
    $run = Load-Run
    Check-Health $run
    Copy-Diagnostics $run
    $log = Join-Path $DiagDir 'logs\kinolune.log'
    if (Test-Path $log) { Get-Content -Encoding UTF8 -Tail ([int](Opt 'tail' 30)) $log } else { Write-Output 'no log (diagnostics off?)' }
}

function Write-Report($run) {
    $lines = @(Get-Content -Encoding UTF8 (Join-Path $run.dir 'run.log'))
    $count = { param($k) @($lines | Where-Object { $_ -match "^\S+ $k\s" }).Count }
    $mem = Join-Path $run.dir 'memory.tsv'
    $memLine = 'нет замеров'
    if (Test-Path $mem) {
        $m = @(Get-Content $mem | ForEach-Object { [int](($_ -split "`t")[1]) })
        if ($m.Count) { $memLine = "private bytes: в начале $($m[0]) МБ, в конце $($m[-1]) МБ, максимум $(($m | Measure-Object -Maximum).Maximum) МБ ($($m.Count) замеров)" }
    }
    $findings = Join-Path $run.dir 'findings.md'
    $r = @(
        "# Живой прогон Kinolune на Windows $($run.id)", '',
        "- Сборка: Kinolune $($run.version) (`$(Split-Path -Leaf $run.exe)`), $(if ($run.fresh) { 'чистый профиль' } else { 'профиль пользователя' })",
        "- Система: $($run.os)",
        "- Начало: $($run.started)",
        "- Действий: клики $(& $count 'CLICK'), клавиши $(& $count 'KEY'), ввод $(& $count 'TYPE'), шаги $(& $count 'STEP'), скриншоты $($run.shots)",
        "- Падения: $(& $count 'CRASH'), неудачные ожидания: $(& $count 'FAIL'), ошибки и предупреждения журнала: $(& $count 'LOGERR')",
        "- Память: $memLine",
        "- Находок: $($run.notes)$(if ($run.notes) { ' — findings.md' })", '',
        '## Падения и ошибки журнала', ''
    )
    $r += @($lines | Where-Object { $_ -match '^\S+ (CRASH|FAIL|LOGERR)\s' } | Select-Object -First 60 | ForEach-Object { "    $_" })
    if (Test-Path $findings) { $r += @('', '## Находки', '') + @(Get-Content -Encoding UTF8 $findings | Select-Object -Skip 3) }
    $r | Set-Content -Encoding UTF8 (Join-Path $run.dir 'report.md')
}

function Cmd-Report { $run = Load-Run; Check-Health $run; Write-Report $run; Get-Content -Encoding UTF8 (Join-Path $run.dir 'report.md') }

switch ($Command.ToLowerInvariant()) {
    'start' { Cmd-Start }
    'stop' { Cmd-Stop }
    'restart' { Cmd-Restart }
    'restore' { Cmd-Restore }
    'status' { Cmd-Status }
    'tree' { Cmd-Tree }
    'texts' { Cmd-Texts }
    'find' { Cmd-Find }
    'click' { Cmd-Click }
    'clickxy' { Cmd-ClickXY }
    'act' { Cmd-Act }
    'type' { Cmd-Type }
    'key' { Cmd-Key }
    'scroll' { Cmd-Scroll }
    'wait' { Cmd-Wait }
    'shot' { Cmd-Shot }
    'explore' { Cmd-Explore }
    'note' { Cmd-Note }
    'logs' { Cmd-Logs }
    'report' { Cmd-Report }
    default { Get-Help $PSCommandPath -Detailed | Out-String | Write-Output }
}
