#requires -version 5
<#
Read-only desktop integration diagnostic for Octos.

Run it in all of these states:
  1. Octos main window foreground
  2. Octos main window minimized
  3. Show desktop (Win+D / ToggleDesktop)

Expected in every state:
  - SysListView32 (desktop icons) visible
  - when the wallpaper is visible: parent=Progman, prev=SHELLDLL_DefView
    (when fullscreen hides it, the z-order handles may be empty)
  - msedgewebview2 Chrome_WidgetWin_* overlay is layered+transparent (per-pixel alpha)
  - desktop right-click opens the context menu
  - clicking a wallpaper button produces a `wallpaper page:` log entry

Important: this script cannot detect whether the code called
SetLayeredWindowAttributes(alpha=255) on a Chrome overlay. That call must never be
used; it makes the WebView2 host opaque and covers icons. See docs/upstream-map.md.

Usage (Windows PowerShell):
  powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-desktop.ps1

It does not click, show, hide, or modify anything.
#>

$hideIcons = $null
try {
    $hideIcons = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced' -Name HideIcons -ErrorAction Stop).HideIcons
} catch {
    $hideIcons = 'missing'
}
Write-Output "HideIcons=$hideIcons (0 means desktop icons are enabled)"

Add-Type @'
using System;
using System.Text;
using System.Diagnostics;
using System.Runtime.InteropServices;

public class OctosDesktopCheck {
    public delegate bool EnumProc(IntPtr h, IntPtr l);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowW(string c, string n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr p, IntPtr a, string c, string n);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint f);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint f);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int L, T, R, B; }

    static bool wallpaperBelowIcons = false;
    static bool wallpaperVisible = false;
    static bool mainVisible = false;
    static bool mainMinimized = false;

    static string Cls(IntPtr h) {
        var b = new StringBuilder(256);
        GetClassName(h, b, 256);
        return b.ToString();
    }

    static string Proc(IntPtr h) {
        uint pid;
        GetWindowThreadProcessId(h, out pid);
        try {
            return Process.GetProcessById((int)pid).ProcessName + "/" + pid;
        } catch {
            return "pid=" + pid;
        }
    }

    static string Rect(IntPtr h) {
        RECT r;
        GetWindowRect(h, out r);
        return r.L + "," + r.T + " " + r.R + "," + r.B;
    }

    static string ExStyle(IntPtr h) {
        long ex = GetWindowLongPtr(h, -20).ToInt64();
        return "0x" + ex.ToString("X") +
            " layered=" + ((ex & 0x00080000) != 0) +
            " transparent=" + ((ex & 0x00000020) != 0) +
            " toolwindow=" + ((ex & 0x00000080) != 0) +
            " noactivate=" + ((ex & 0x08000000) != 0);
    }

    static bool LayeredTransparent(IntPtr h) {
        long ex = GetWindowLongPtr(h, -20).ToInt64();
        return (ex & 0x00080000) != 0 && (ex & 0x00000020) != 0;
    }

    // Hidden IME/helper windows can sit between the WebView2 host and Progman
    // without affecting what the user sees. Walk down through invisible windows
    // and fail only if a visible window is above the desktop.
    static bool ReachesProgman(IntPtr h, IntPtr progman) {
        for (int i = 0; i < 16; i++) {
            h = GetWindow(h, 2);
            if (h == IntPtr.Zero) { return false; }
            if (h == progman) { return true; }
            if (IsWindowVisible(h)) { return false; }
        }
        return false;
    }

    static void Walk(IntPtr h) {
        string c = Cls(h);
        if (c == "OctosMainWindow") {
            mainVisible = IsWindowVisible(h);
            mainMinimized = IsIconic(h);
            IntPtr prev = GetWindow(h, 3);
            IntPtr next = GetWindow(h, 2);
            Console.WriteLine(
                "  " + c + " hwnd=" + h +
                " proc=" + Proc(h) +
                " visible=" + mainVisible +
                " minimized=" + mainMinimized +
                " rect=" + Rect(h) +
                " ex=" + ExStyle(h) +
                " prev=" + prev + "(" + Cls(prev) + ")" +
                " next=" + next + "(" + Cls(next) + ")");
        } else if (c == "OctosWallpaperWindow") {
            IntPtr parent = GetAncestor(h, 1);
            IntPtr prev = GetWindow(h, 3);
            IntPtr next = GetWindow(h, 2);
            wallpaperVisible = IsWindowVisible(h);
            bool below = Cls(prev) == "SHELLDLL_DefView";
            if (below) {
                wallpaperBelowIcons = true;
            }
            Console.WriteLine(
                "  " + c + " hwnd=" + h +
                " proc=" + Proc(h) +
                " visible=" + IsWindowVisible(h) +
                " rect=" + Rect(h) +
                " ex=" + ExStyle(h) +
                " parent=" + parent + "(" + Cls(parent) + ")" +
                " prev=" + prev + "(" + Cls(prev) + ")" +
                " next=" + next + "(" + Cls(next) + ")" +
                " belowIcons=" + below);
        }
        EnumChildWindows(h, (ch, l) => { Walk(ch); return true; }, IntPtr.Zero);
    }

    public static void Run() {
        IntPtr progman = FindWindowW("Progman", null);
        IntPtr shell = FindWindowExW(progman, IntPtr.Zero, "SHELLDLL_DefView", null);
        IntPtr listview = shell == IntPtr.Zero ? IntPtr.Zero : FindWindowExW(shell, IntPtr.Zero, "SysListView32", null);
        bool iconsVisible = IsWindowVisible(listview);

        Console.WriteLine("Progman=" + progman + " proc=" + Proc(progman) + " visible=" + IsWindowVisible(progman));
        Console.WriteLine("SHELLDLL_DefView=" + shell + " visible=" + IsWindowVisible(shell));
        Console.WriteLine("SysListView32=" + listview + " visible=" + iconsVisible);

        Console.WriteLine("Octos windows:");
        EnumWindows((h, l) => { Walk(h); return true; }, IntPtr.Zero);

        int screenWidth = GetSystemMetrics(0);
        int screenHeight = GetSystemMetrics(1);
        int chromeOverlays = 0;
        int otherChromeWindows = 0;
        int chromeNextProgman = 0;
        int chromeLayeredTransparent = 0;

        Console.WriteLine("Visible full-screen Chrome_WidgetWin_* top-level overlays:");
        EnumWindows((h, l) => {
            string c = Cls(h);
            if (!c.StartsWith("Chrome_WidgetWin_") || !IsWindowVisible(h)) {
                return true;
            }

            RECT r;
            GetWindowRect(h, out r);
            bool fullScreen = r.L <= 2 && r.T <= 2 && r.R >= screenWidth - 2 && r.B >= screenHeight - 2;
            if (!fullScreen) {
                return true;
            }

            string proc = Proc(h);
            bool isWebView2 = proc.StartsWith("msedgewebview2/");
            IntPtr prev = GetWindow(h, 3);
            IntPtr next = GetWindow(h, 2);
            bool immediateProgman = next == progman;
            bool nextProgman = ReachesProgman(h, progman);
            bool layeredTransparent = LayeredTransparent(h);
            if (isWebView2) { chromeOverlays++; } else { otherChromeWindows++; }
            if (isWebView2 && nextProgman) { chromeNextProgman++; }
            if (isWebView2 && layeredTransparent) { chromeLayeredTransparent++; }

            Console.WriteLine(
                "  " + h + " " + c +
                " proc=" + proc +
                " rect=" + Rect(h) +
                " ex=" + ExStyle(h) +
                " prev=" + prev + "(" + Cls(prev) + ")" +
                " next=" + next + "(" + Cls(next) + ")" +
                " nextIsProgman=" + immediateProgman +
                " reachesProgman=" + nextProgman +
                " layeredTransparent=" + layeredTransparent +
                " webview2=" + isWebView2);
            return true;
        }, IntPtr.Zero);

        Console.WriteLine(
            "SUMMARY iconsVisible=" + iconsVisible +
            " wallpaperBelowIcons=" + wallpaperBelowIcons +
            " wallpaperVisible=" + wallpaperVisible +
            " mainVisible=" + mainVisible +
            " mainMinimized=" + mainMinimized +
            " chromeOverlays=" + chromeOverlays +
            " otherChromeWindows=" + otherChromeWindows +
            " chromeNextProgman=" + chromeNextProgman +
            " chromeLayeredTransparent=" + chromeLayeredTransparent);

        if (!iconsVisible || (wallpaperVisible && !wallpaperBelowIcons)) {
            Console.WriteLine("WARN: expected iconsVisible=True and (when visible) wallpaperBelowIcons=True.");
        } else if (wallpaperVisible && chromeOverlays == 0) {
            Console.WriteLine("WARN: wallpaper window is visible but no full-screen WebView2 Chrome host exists;");
            Console.WriteLine("      the gradient may not render (controller/bounds lifecycle issue).");
        } else if (!wallpaperVisible) {
            Console.WriteLine("PASS: wallpaper is hidden (fullscreen); desktop icons are visible.");
        } else {
            Console.WriteLine("PASS: wallpaper is below the icon layer and desktop icons are visible.");
        }

        Console.WriteLine("NOTE: uniform alpha on a Chrome overlay cannot be detected via Win32.");
        Console.WriteLine("      Never call SetLayeredWindowAttributes(alpha=255); see docs/upstream-map.md.");
        Console.WriteLine("Manual: right-click desktop, click a wallpaper button, inspect octos.log for `wallpaper page:`.");
    }
}
'@

[OctosDesktopCheck]::Run()
