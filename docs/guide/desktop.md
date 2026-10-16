# Desktop app

AGT Studio is the agt console as a desktop app for macOS, Windows and Linux. It shows the same pages as `agt web`, but talks to agt inside the app instead of over a local web server, keeps running in the system tray, and notifies you when something needs you.

## Install

Download the installer for your system from the [latest release](https://github.com/blacsheep/agt/releases/latest):

| System | File |
| --- | --- |
| macOS (Apple silicon and Intel) | `AGT.Studio_<version>_universal.dmg` |
| Windows | `AGT.Studio_<version>_x64-setup.exe` or `.msi` |
| Linux | `.AppImage`, `.deb` or `.rpm` |

On Linux the app needs WebKitGTK 4.1 (`libwebkit2gtk-4.1-0`), which the `.deb` and `.rpm` packages pull in.

::: warning Unsigned builds
Until the project signs its builds, macOS asks you to confirm the first time you open the app (right-click the app, then **Open**), and Windows SmartScreen may show a warning (**More info** → **Run anyway**).
:::

## What it uses

- **The same home as the CLI:** `$AGT_HOME`, or `~/.agt`. Resources, agents and conversations you made with `agt` or `agt web` appear in the app, and the other way round.
- **The same identity as `agt web`** (`web-operator`), so both can manage one workspace.
- **One executable.** The app also runs the agt commands its pages need, so nothing else has to be installed. You can still install the `agt` CLI separately for the terminal.

Run either `agt web` or the app for a home, not both at once: both start the home's scheduler, and only one can own it.

## Tray and notifications

Closing the window keeps AGT Studio in the system tray, so agents keep answering and timers keep firing. Click the tray icon to bring the window back; choose **Quit** in its menu to stop the app and its scheduler.

While the window is in the background, the app notifies you when:

- an agent posts a message in one of your conversations;
- a [timer](./timers.md) fires;
- a request waits for your [approval](./permissions.md).

Your system may ask once whether AGT Studio may show notifications.

## Updates

When a release is published with update signatures, the app downloads and installs the update at startup and tells you; it takes effect the next time you open it. Releases without signatures are installed by downloading the new installer.

## Build from source

```bash
cargo install tauri-cli --version "^2.12" --locked
cd crates/agt-desktop
cargo tauri dev       # run with live reload of web/public
cargo tauri build     # make installers under target/release/bundle
```

On Linux, install the WebView and tray libraries first, for example on Ubuntu:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```
