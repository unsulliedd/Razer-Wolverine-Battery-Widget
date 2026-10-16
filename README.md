# Razer Wolverine Battery Widget

See your **Razer Wolverine V3 Pro** battery level in the Windows tray, without
installing Synapse or Razer Controller Setup. Works in both the controller's
**Xbox mode** and **PC mode**, on the cable or the HyperSpeed dongle.

> **Not affiliated with, endorsed by, or sponsored by Razer Inc.** "Razer" and
> "Wolverine" are trademarks of Razer Inc., used here only to say which
> hardware this tool works with.

## Download

Grab `WolverineBatteryWidget.exe` from the
[latest release](../../releases/latest) and run it. There is no installer: put
the .exe wherever you like and delete it when you're done with it.

The binary is not code signed, so Windows SmartScreen will say the publisher is
unknown the first time you run it. Choose **More info → Run anyway**, or check
it against `SHA256SUMS.txt` from the same release first.

## What it does

**Tray icon.** A controller that fills from the left with the charge: green,
amber at 20% or below, red at 10% or below, with a bolt while charging. Turn on
**Show percentage in tray** to see the number under a small controller instead.
Hover the icon for the exact level and how old the reading is.

**Panel.** Click the tray icon for the battery percentage, charging state,
cable or dongle, the charge rate with time to full, and a history graph. The
buttons at the top right are:

- **Background reading** (clock): green with a dot when on. See below.
- **Refresh:** reads the battery now.
- **Settings** (gear): Start with Windows, Show percentage in tray, Low battery
  alerts, Quit. Right-clicking the tray icon opens settings too.

Click the footer to see how often the battery is being read.

**Low battery alerts.** A Windows notification at 20% and again at 10%, never
while charging. They re-arm once the battery is back above 25%. On by default;
turn them off in settings.

**When the controller is off,** the panel keeps the last reading as a dimmed
*Disconnected* card, so you can still see where it was.

### How often it reads

| | Panel open | Panel closed |
|---|---|---|
| **Xbox mode** | every 3 s | no readings |
| **PC mode** | every 3 s | every 30 s, if **background reading** is on |

Xbox mode can only read while the panel is open and focused: Windows passes the
controller's replies only to the app in the foreground. PC mode has no such
limit, so background reading (off by default) keeps the tray and the alerts
current while you play.

### A note on accuracy

The controller estimates its charge from the battery voltage, and that reads
high while it is on the cable. So while charging the panel shows the level as
an estimate (`~73%`), and you will see it drop a few percent in the first
minutes after unplugging. The resting value, about 15 minutes off the cable, is
the one to trust.

## Start with Windows

Open settings and tick **Start with Windows**. It adds a per-user registry
entry that starts the widget when you sign in. Untick it to remove the entry.

## How it works

The widget sends the controller the same two read requests Razer's own software
uses, one for the battery level and one for the charging state, and nothing
else. A safety check in the code refuses to build any other request, so the
widget can never change a setting on the controller.

- **Xbox mode:** the requests go through the Windows controller interface
  (`Windows.Gaming.Input.Custom`) as Xbox controller messages.
- **PC mode:** the controller appears as a regular PC gamepad, which that
  interface can't reach. The requests go to its Razer settings channel instead,
  as a HID feature report. Synapse uses this channel too, and the two share it
  safely.

Each reply's command, transaction ID and checksum are checked before the raw
0–255 value is turned into a percentage.

## Requirements

- **Windows 10 version 1709 (build 16299) or newer**, or Windows 11, 64-bit.
- A **Razer Wolverine V3 Pro**, on the cable or its HyperSpeed dongle, in Xbox
  mode (USB ID `1532:0A3F`) or PC mode (`1532:0A4C`).

## License

MIT — see [LICENSE](LICENSE).
