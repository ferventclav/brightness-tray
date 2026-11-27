# Brightness Tray

A lightweight, native Rust Windows system tray app to control brightness across both **internal laptop displays** (via WMI `WmiMonitorBrightnessMethods`) and **external monitors** (via DDC/CI `dxva2`).

Built as a standalone replacement for the PowerToys brightness flyout with zero runtime dependencies (~340 KB binary).

![Brightness Tray flyout preview](docs/preview.png)

## Context Menu & Controls

Open the Fluent 2 Acrylic menu by **right-clicking** the tray icon or clicking the **Settings** gear in the flyout:

| Tray Right-Click Menu | Flyout Settings Menu |
| :---: | :---: |
| ![Tray context menu](docs/menu-tray.png) | ![Flyout settings menu](docs/menu-flyout.png) |

- **Match room light (Auto):** Takes a one-shot ambient light reading (using a hardware `LightSensor` if available, or your webcam in read-only mode) and smoothly adjusts all displays. Automatically hidden on PCs with neither a light sensor nor a webcam.
- **Auto-adjust on wake:** Runs a one-shot room-light adjustment after waking from sleep or opening the laptop lid. Starting the app or signing in does not trigger it.
- **Link monitors:** Keeps brightness deltas synchronized across all connected displays when you drag any slider.
- **Run at Windows startup:** Registers the app to start automatically when you log in to Windows.
- **Refresh monitors:** Re-scans connected internal and external `DDC/CI` displays.
- **Exit:** Closes `brightness-tray` cleanly.

## Quick Start

1. **Download:** Grab `brightness-tray.exe` from the [latest GitHub Release](https://github.com/ferventclav/brightness-tray/releases/latest) and place it anywhere you like (e.g. `C:\Tools\brightness-tray.exe`).
2. **Run:** Double-click `brightness-tray.exe`. Look for the brightness icon in your system tray (or inside the `^` overflow menu, then drag it onto the taskbar). Running the exe again while it is already running opens the flyout.
3. **Adjust brightness:**
   - **Left-click** the tray icon (or press `Win+B`, select it, and hit `Enter`) to open the flyout. Opened from the keyboard, it appears beside the tray icon wherever the mouse is.
   - **Drag** a slider or **scroll** over any monitor card (`±5%` per wheel notch, or hold `Shift` for `±1%`). A touchpad swipe moves it at the same rate as a mouse notch.
   - **Keyboard:** Use `Up`/`Down` or `Tab` to select a display, `Left`/`Right` to adjust brightness (`Shift` for `±1%`), `PageUp`/`PageDown` for `±10%`, `Home`/`End` for `0%`/`100%`, `A` for Auto, `L` to Link, `R` to Refresh, and `Esc` to close. `Tab` continues past the displays into the toolbar (`Left`/`Right` move along it, `Enter` or `Space` presses a button), and the `Menu` key opens Settings.
   - Click **Auto** *(shown when your PC has a light sensor or webcam)* to match your room lighting.
   - Click the **Link** icon to adjust all monitors together.
   - Click **Refresh** if you plug in or wake an external monitor.
4. **Start with Windows:** Right-click the tray icon (or click the **Settings** gear in the flyout) and check **Run at Windows startup**. If you later move the exe, run it once from its new folder and the startup entry follows it. Switching the app off in Task Manager's Startup tab shows here as unchecked.

> **Note for external monitors:** Make sure **DDC/CI** is enabled in your monitor's on-screen display (OSD) menu so Windows can control its hardware backlight.

## Features

- **Multi-monitor support:** Automatically discovers internal laptop panels (`eDP` / `LVDS` / `Internal`) and external `DDC/CI` monitors with friendly names from `QueryDisplayConfig`.
- **Windows 11 Acrylic flyout:** Direct3D 11 + DirectComposition + Direct2D UI that adapts to your Windows dark/light theme and accent color, with rounded corners and per-monitor DPI scaling.
- **Smooth transitions:** Non-blocking background worker thread with preemptible `700ms` ease-in-out cubic hardware ramping and 60fps slider animation.
- **Link monitors:** Toggle delta-synced brightness across all connected displays.
- **Adaptive Auto-Brightness (3-tier hardware fallback):**
  1. Uses a dedicated hardware **Ambient Light Sensor** (`Windows.Devices.Sensors.LightSensor`) when available (zero camera usage).
  2. Falls back to **on-demand webcam room-light sampling** (read-only, using a Weber-Fechner perceptual curve). If another app is using the camera, Auto leaves brightness alone and shows "Couldn't open camera"; the other app keeps its video.
  3. On desktops/laptops with neither an ALS nor a webcam, cleanly hides the **Auto** controls so only manual sliders, **Link**, **Refresh**, and **Settings** are shown.
- **Resilient tray icon:** Re-registers on `TaskbarCreated` and retries at logon if Explorer is still starting up, plus **Run at Windows startup** toggle.

## Requirements

- Windows 10 or Windows 11 (x64)
- External monitors must have **DDC/CI** enabled in their OSD settings to be controllable.

## Build from Source

```powershell
cargo build --release
```

The standalone executable is output at `target\release\brightness-tray.exe`.
