"""Put a "Desktop app" download table at the top of a release's notes.

Usage: desktop-notes.py <assets.json> <body.md>  (prints the new body)
Rows link only to installers that are attached; rerunning replaces the table.
"""
import json
import re
import sys

START, END = "<!-- desktop-app -->", "<!-- /desktop-app -->"
ROWS = [
    ("macOS (Apple silicon and Intel)", r"_universal\.dmg$"),
    ("Windows", r"_x64-setup\.exe$"),
    ("Windows (MSI)", r"_x64_.*\.msi$"),
    ("Linux (AppImage)", r"\.AppImage$"),
    ("Linux (Debian, Ubuntu)", r"\.deb$"),
    ("Linux (Fedora, openSUSE)", r"\.rpm$"),
]

assets = json.load(open(sys.argv[1]))
body = open(sys.argv[2]).read()
body = re.sub(re.escape(START) + r".*?" + re.escape(END) + r"\n*", "", body, flags=re.S)
rows = []
for system, pattern in ROWS:
    for asset in assets:
        if asset["name"].startswith("AGT.Studio") and re.search(pattern, asset["name"]):
            rows.append(f"| {system} | [{asset['name']}]({asset['url']}) |")
            break
if rows:
    table = "\n".join(
        [
            START,
            "## Desktop app",
            "",
            "AGT Studio, the console as a desktop app. Download the installer for your system:",
            "",
            "| System | Installer |",
            "| --- | --- |",
            *rows,
            "",
            "The installers are not signed yet; see "
            "[Desktop app](https://blacsheep.github.io/agt/guide/desktop) for opening them the first time. "
            "For the `agt` command line, use the install commands below.",
            END,
            "",
            "",
        ]
    )
    body = table + body
sys.stdout.write(body)
