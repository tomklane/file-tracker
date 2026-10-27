FILE TRACKER
============
Scans a folder (Finder on Mac, File Explorer on Windows) and fills a tracking
spreadsheet with the files it finds. It reads the columns your spreadsheet
already has and lets you choose what goes in each one, so it works with any
folder layout and any spreadsheet layout.


ONE-TIME SETUP
--------------
1. Install Python 3 from https://www.python.org/downloads/
   - Windows: tick "Add python.exe to PATH" in the installer.
   - Mac: use the python.org installer (Homebrew's Python lacks the window toolkit).
2. Start the app:
   - Mac:     double-click "Start File Tracker (Mac).command"
              (first time: right-click it > Open > Open, because it was downloaded)
   - Windows: double-click "Start File Tracker (Windows).bat"
   On first launch it installs one free helper package (openpyxl), so be online.

Mac note: the first time you scan Documents, Desktop or Downloads, macOS asks
whether Python may access that folder. Click Allow.


USING IT
--------
1  Folder to scan   Pick the top folder. Optionally limit to file types
                    (pdf, docx) or skip names (*.tmp, Archive, Old*).
2  Spreadsheet      Pick your existing tracker (.xlsx, .xlsm or .csv). The app
                    finds the header row itself, even below a title, and you can
                    change the row number. Or choose "Create a new one" and type
                    the column names you want.
3  Each column      Choose what fills it: file name, part of the file name
                    (e.g. PRJ001_Beam-Layout_RevA -> part 1 = PRJ001, part -1 = RevA),
                    folder at a given level, path, clickable link, size, dates,
                    a running ID like DOC-###, Present/Missing, or fixed text.
                    Leave your own columns (Status, Notes, Reviewer...) on
                    "leave as is" and they are never touched.
                    "Match files using column" is how the app recognises a file
                    that's already listed. Full path or Link is safest.
4  Copy or update   EITHER click "Show list to copy" to open a window with the
                    files laid out in your spreadsheet's column order. Click
                    "Copy all" (or select rows and press Ctrl+C / Cmd+C), then
                    click the first empty cell in your spreadsheet and paste.
                    Works with Excel, Numbers and Google Sheets. Nothing is saved
                    to your file, and by default only files not yet in your sheet
                    are listed. Links paste as clickable HYPERLINK formulas.
                    OR click "Update spreadsheet" to have the app write straight
                    into the file. Run it again any time: new files
                    are added at the bottom, existing rows are refreshed, and
                    deleted files are flagged if you mapped a Present/Missing column.

Your setup is remembered between runs. Use "Save setup..." to keep separate
setups for different projects or spreadsheets, and "Load setup..." to switch.


GOOD TO KNOW
------------
- A backup of the spreadsheet is saved in a "_tracker_backups" folder next to it
  before every update (last 20 kept).
- Close the spreadsheet in Excel before updating, or the save will fail.
- New rows copy the formatting of the first data row. Excel tables and filters
  are extended to include them.
- The app keeps formulas, formatting, validation and other sheets, but Excel
  cannot keep pictures or charts through this kind of edit, so keep those on a
  separate copy if your tracker has them.
- Old .xls and Apple Numbers files: save/export them as .xlsx first.


OPTIONAL: MAKE IT A NORMAL APP (no Python needed to run it)
-----------------------------------------------------------
Open Terminal (Mac) or Command Prompt (Windows) in this folder and run:
    python3 -m pip install pyinstaller openpyxl          (Windows: py -m pip ...)
    python3 -m PyInstaller --windowed --onefile --name "File Tracker" file_tracker.py
The finished app appears in the "dist" folder. Build it on each system you need
(build on a Mac for Mac, on Windows for Windows).
