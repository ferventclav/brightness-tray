# Brightness Tray

A lightweight, native Rust Windows system tray app to control brightness across both **internal laptop displays** (via WMI `WmiMonitorBrightnessMethods`) and **external monitors** (via DDC/CI `dxva2`).

Built as a standalone replacement for the PowerToys brightness flyout with zero runtime dependencies. It ships as one 421 KB binary.

![Brightness Tray flyout preview](docs/preview.png)

## Context Menu & Controls

Open the Fluent 2 Acrylic menu by **right-clicking** the tray icon or clicking the **Settings** gear in the flyout:

| Tray Right-Click Menu | Flyout Settings Menu |
| :---: | :---: |
| ![Tray context menu](docs/menu-tray.png) | ![Flyout settings menu](docs/menu-flyout.png) |

The popup menu holds six items. **Match room light (Auto)** takes a one-shot ambient light reading (using a hardware `LightSensor` if available, or your webcam in read-only mode) and adjusts every screen; PCs with neither sensor hide it automatically. **Auto-adjust on wake** runs the same reading after sleep or opening the laptop lid, while startup and sign-in leave it alone. **Link displays** keeps brightness deltas synchronized across connected screens during any slider drag. **Run at Windows startup** registers logon launch, **Refresh displays** re-scans internal and external DDC/CI hardware, and **Exit** stops the tray app.

## Quick Start

1. Download `brightness-tray.exe` from the [latest GitHub Release](https://github.com/ferventclav/brightness-tray/releases/latest) and save it in any folder, for example `C:\Tools\`.
2. Double-click it. The brightness icon appears in the system tray, or in the `^` overflow menu until you drag it onto the taskbar. Running the exe again while it is open brings up the flyout instead of a second copy.
3. Open the flyout:
   - Left-click the tray icon.
   - Or press `Win+B`, arrow to the icon and press `Enter`. Opened from the keyboard, the flyout appears beside the tray icon wherever the mouse is.
4. Adjust brightness:
   - Drag a slider, or scroll over a display card: `±5%` per notch, `±1%` with `Shift`. A touchpad swipe moves at the same rate as a mouse notch.
   - `Up`/`Down` pick a display. `Left`/`Right` change it by 5% (`Shift` for 1%), `PageUp`/`PageDown` by 10%, and `Home`/`End` jump to 0% or 100%.
   - `Tab` moves through the displays and then the toolbar buttons. `Enter` or `Space` presses a button, and `Esc` closes the flyout.
   - Shortcuts: `A` for Auto, `L` for Link, `R` for Refresh, and the `Menu` key for Settings.
5. Use the toolbar:
   - **Auto** matches your room light. It shows only when your PC has a light sensor or a webcam, and says "Camera in use" if another app has the camera.
   - **Link** moves all displays together. It is turned off when only one display is connected.
   - **Refresh** re-scans displays after you plug in or wake a monitor.
6. To start with Windows, open Settings (the gear, or right-click the tray icon) and check **Run at Windows startup**. If you move the exe later, run it once from its new folder and the startup entry follows it. Turning the app off in Task Manager's Startup tab shows here as unchecked.

> **Note for external monitors:** Enable **DDC/CI** in your monitor's on-screen display (OSD) menu so Windows can control its hardware backlight.

## Features

- Discover internal laptop panels (`eDP` / `LVDS` / `UDI` embedded) and external DDC/CI monitors with friendly names from `QueryDisplayConfig`.
- Render a Direct3D 11 + DirectComposition + Direct2D Acrylic flyout and menu with Segoe Fluent Icons (falling back to Segoe MDL2 Assets on Windows 10), Windows dark/light theme and accent matching, rounded corners, and per-monitor DPI scaling.
- Expose Windows UI Automation (`RangeValue`, `Toggle`, `Invoke`, focus tracking, and status announcements) for Narrator and screen readers.
- Apply changes on a background worker thread with preemptible hardware ramping and 60fps UI slider interpolation.
- Sync brightness deltas across screens via **Link displays**.
- Measure room light in three tiers. It checks a dedicated Windows `LightSensor` first with zero camera access, falls back to read-only Media Foundation webcam sampling (checking active camera use via `IMFSensorActivityMonitor` first and showing `Camera in use` or `Couldn't open camera` instead of touching a busy device), and hides Auto controls when neither hardware sensor exists.
- Recover the tray icon across `TaskbarCreated` restarts and Explorer startup delays.

## Requirements

- Windows 10 or Windows 11 (x64).
- External monitors need **DDC/CI** enabled in their OSD settings.

## Build from Source

```powershell
cargo build --release
```

The standalone executable lands at `target\release\brightness-tray.exe`. It has a single static binary and needs no external runtime.
