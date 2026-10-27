#!/bin/bash
# Double-click to start File Tracker on a Mac.
cd "$(dirname "$0")"

# Prefer the python.org install (it has a modern, reliable window toolkit).
for py in \
  "/Library/Frameworks/Python.framework/Versions/Current/bin/python3" \
  /usr/local/bin/python3 \
  /opt/homebrew/bin/python3 \
  python3; do
  if command -v "$py" >/dev/null 2>&1 && "$py" -c "import tkinter" >/dev/null 2>&1; then
    "$py" file_tracker.py &
    disown
    sleep 1
    osascript -e 'tell application "Terminal" to close (every window whose name contains "Start File Tracker")' >/dev/null 2>&1 &
    exit 0
  fi
done

osascript -e 'display dialog "File Tracker needs Python 3.\n\nClick OK to open python.org, download the macOS installer, install it, then double-click Start File Tracker again." buttons {"OK"} default button 1 with title "File Tracker"'
open "https://www.python.org/downloads/macos/"
