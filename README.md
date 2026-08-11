# Razer Wolverine Battery Widget

See your **Razer Wolverine V3 Pro** battery level in the Windows tray, without
installing Synapse or Razer Controller Setup.

> **Not affiliated with, endorsed by, or sponsored by Razer Inc.** "Razer" and
> "Wolverine" are trademarks of Razer Inc., used here only to say which
> hardware this tool works with.

## Download

Grab `WolverineBatteryWidget.exe` from the
[latest release](../../releases/latest) and run it. There is no installer and
nothing to uninstall — put the file wherever you keep small tools and delete it
when you are done with it.

The binary is not code signed, so Windows SmartScreen will say the publisher is
unknown the first time you run it. Choose **More info → Run anyway**, or check
the file against `SHA256SUMS.txt` on the release page first.

## Start with Windows

Right-click the tray icon and tick **Start with Windows**. It adds a per-user
registry entry that launches the app when you sign in — no admin rights, no
service, nothing running before you log on. Untick it to remove the entry.

**Move the exe somewhere permanent before you tick it.** The entry records the
full path, so enabling it while the file is still in your Downloads folder
means autostart breaks the moment that folder gets cleaned out. If you do move
the file later, launch it once by hand and it repairs the entry itself.

## What it does

Click the tray icon and a panel shows the battery percentage, whether it is
charging, whether it is on the cable or the HyperSpeed dongle, and a history
graph. The tray icon itself is a battery glyph that fills to match, redrawn for
your DPI and for light or dark taskbars.


## Requirements

- **Windows 10 version 1709 (build 16299) or newer**, or Windows 11, 64-bit.
- A **Razer Wolverine V3 Pro** (USB ID `1532:0A3F`), on the cable or its
  dongle.


## License

MIT — see [LICENSE](LICENSE).
