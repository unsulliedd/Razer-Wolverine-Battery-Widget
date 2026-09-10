# Razer Wolverine Battery Widget

See your **Razer Wolverine V3 Pro** battery level in the Windows tray, without
installing Synapse or Razer Controller Setup.

> **Not affiliated with, endorsed by, or sponsored by Razer Inc.** "Razer" and
> "Wolverine" are trademarks of Razer Inc., used here only to say which
> hardware this tool works with.

## Download

Grab `WolverineBatteryWidget.exe` from the
[latest release](../../releases/latest) and run it. There is no installer, just put the .exe wherever and delete it once you're done with it.

The binary is not code signed, so Windows SmartScreen will say the publisher is
unknown the first time you run it. Choose **More info → Run anyway**.

## Start with Windows

Right-click the tray icon and tick **Start with Windows**. It adds a per-user
registry entry that launches the app when you sign in. Untick it to remove the entry.

## What it does

Click the tray icon and a panel shows the battery percentage, whether it is
charging, whether it is on the cable or the HyperSpeed dongle, and a history
graph. 

Due to Microsoft being a hard-ass it only works when the widget is open and in focus, background battery requests get ignored unfortunately.

<img width="499" height="469" alt="afbeelding" src="https://github.com/user-attachments/assets/3de6d889-243a-40c3-9c3a-0ab91bcfc845" />


## Requirements

- **Windows 10 version 1709 (build 16299) or newer**, or Windows 11, 64-bit.
- A **Razer Wolverine V3 Pro** (USB ID `1532:0A3F`), on the cable or its
  dongle.


## License

MIT — see [LICENSE](LICENSE).
