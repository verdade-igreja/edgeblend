# Autostart

Runs edgeblend at logon through a Scheduled Task, supervised by `--watchdog` (restarts after
a crash with 2 s, 4 s ... 60 s backoff; stops when the app is quit from the tray).

```powershell
.\install-autostart.ps1                 # dry run
.\install-autostart.ps1 -Apply          # register task, remove edgeblend.lnk from Startup if present
.\install-autostart.ps1 -Uninstall -Apply
```

`-ExePath` defaults to `%LOCALAPPDATA%\Programs\edgeblend\edgeblend.exe`. No admin rights needed.

## Check

- `Get-ScheduledTask edgeblend | Get-ScheduledTaskInfo`
- Log (fatal errors, panics, watchdog restarts): `%APPDATA%\edgeblend\edgeblend.log`
- Stop for good: tray icon > Quit (the watchdog exits too). Closing the window only hides it.

## Why not a Windows Service

Services run in session 0, which has no user desktop. Desktop Duplication captures the
logged-on user's desktop and the blend window must be on it, so the app has to run in the
user's interactive session. A logon task does that.
