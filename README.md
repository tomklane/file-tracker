# File Tracker

A desktop app for **macOS and Windows**. It scans a folder (as seen in Finder
or File Explorer) and lays the files out in a **live grid** that matches your
spreadsheet's columns. You select cells in the grid and paste them straight
into Excel, Numbers or Google Sheets. It can also **update your tracking
spreadsheet** (.xlsx, .xlsm, .csv) directly.

It reads the columns your spreadsheet already has, and you choose what goes
in each one. That means it works with any folder layout and any spreadsheet
layout.

## Download

Go to **[Releases](../../releases/latest)** and download:

| Your computer | File |
|---|---|
| Mac (Apple Silicon or Intel) | `File-Tracker-Mac.zip`. Unzip it and drag **File Tracker** to Applications. |
| Windows | `File-Tracker-Windows.exe`. It's a single file with nothing to install. |

**Updates.** From version 1.3.0, File Tracker checks for new versions when it
starts. When one is available, a bar appears at the top: click **Update now**
and it downloads the new version, swaps it in and restarts. You can also click
**Check for updates** at the bottom of the settings, or turn the automatic
check off there. If File Tracker isn't allowed to change the folder it's in
(for example, an admin-only Applications folder), it offers the download
page instead.

**First launch.** The apps aren't signed by Apple or Microsoft, so:
- **Mac:** open the app once. When macOS blocks it, go to **System Settings ›
  Privacy & Security**, scroll down and click **Open Anyway**.
- **Windows:** if SmartScreen appears, click **More info › Run anyway**.

## Using it

1. **Choose a folder.** Click **📁** in the top bar, or drop a folder onto the
   window. The grid fills straight away using standard columns.
2. **Optionally choose your spreadsheet.** Click **📊** (or drop an `.xlsx`
   or `.csv` onto the window) and the grid uses your sheet's own columns,
   column letters and the row numbers new files will land on. Without one,
   you use your own columns, which you can add, rename, move and delete.
3. **Copy from the grid and paste into Excel, Numbers or Google Sheets.**
   Drag across cells (or use Shift+arrows) and press ⌘C / Ctrl+C. **Copy all**
   is in the top bar, and the status bar says which cell to paste at.

The grid updates by itself when you change a setting or when files change.

### In the grid

- **Right-click** a cell, a row number or a column heading for:
  **Show in Finder** (File Explorer on Windows), **Open**, **Copy path**,
  **Copy name**, **Copy with header row**, **Show only rows with this
  value**, **Sort**, **Change column…**, **Leave this column blank**,
  **Skip files/folders named…**, and (with your own columns)
  **Insert / Move / Delete column**. Control-click works as a right-click
  on a Mac.
- **Filter.** Type in the filter box (⌘F / Ctrl+F) to show only rows
  containing that text, in any column. Esc clears it.
- **Sort** with the arrows on a heading: A→Z, Z→A, then back to folder order.
  Numbers sort properly (`Rev2` before `Rev10`).
- When sorting or filtering, running numbers (`DOC-###`) renumber to follow
  what's shown, so a paste stays in sequence.
- **Double-click a heading** to change what fills that column.

| Keys (Mac / Windows) | Does |
|---|---|
| Arrows, Shift+arrows | Move / extend the selection (⌘/Ctrl+arrow jumps to the edge) |
| ⌘C / Ctrl+C | Copy the selection |
| ⇧⌘C / Ctrl+Shift+C | Copy all rows shown |
| ⌘A / Ctrl+A | Select everything |
| ⇧⌘R / Ctrl+Shift+R | Show the selected files in Finder / File Explorer |
| ⌘O / Ctrl+O | Open the selected files |
| ⌘F / Ctrl+F | Filter rows |
| ⌘R / Ctrl+R or F5 | Scan again |
| ⌘, / Ctrl+, | Show or hide the settings |
| Esc | Clear the selection or the filter |

### Settings

Click **Settings** in the top bar (or press ⌘, / Ctrl+,). It has collapsible
sections:

- **Folder & rows.** What each row is (a file, a folder at a level, or each
  folder with files in it), subfolders, hidden files, file types, and names
  to skip.
- **Spreadsheet.** Your sheet's columns or your own, which sheet, the header
  row, and "only list files not yet in the sheet".
- **Columns.** What fills each column. With your own columns you can also
  add, rename, move (↑ ↓) and delete (🗑) them.
- **Options.** Auto-update, size units, how **Update spreadsheet** behaves,
  saved setups, and update checks.

**Update spreadsheet** (or **Save as spreadsheet…** with your own columns)
writes straight into a file instead of copying. New files are added at the
bottom and existing rows are refreshed. Deleted files are flagged if you
mapped a Present/Missing column. A backup goes into `_tracker_backups` first.

### Custom columns: any piece of a folder or file name

Choose **Custom: pick part of a folder or file name…** for a column (or
double-click its heading in the grid). A window opens with a real file from
your folder:

1. **Click where the text comes from.** This can be any folder in the path
   or the file name. Folders can be counted from the top (e.g. the job
   folder) or up from the file (e.g. the folder the file is in), which works
   when folders are nested to different depths.
2. **Choose what to keep:**
   - **A section between separators.** Tick the separators (`_`, `-`,
     space, `.`, or your own) and click the section you want. Shift-click
     takes several sections, and "to the end" takes everything after it.
     "Count from the end" handles names with different numbers of sections.
     Sections can be any length.
   - **Characters by position.** For example, the first 7 characters of
     `2026-03 Smith House` gives `2026-03`. A negative start counts from the
     end.
   - **Text between markers.** For example, after `Rev` and before `_`.
   - **Pattern.** A regular expression, for anything else.
3. **Tidy up.** Trim spaces, change case, or set text to use when nothing is
   found.

The result is shown highlighted, along with what the same rule gives for
other files in your folder, so you can check it before using it. The grid
updates as you go.

For example, with folders like `2026-03 Smith House` and
`2026-11 Big Client Holdings Ltd`, "section 2 to the end, split at space"
gives `Smith House` and `Big Client Holdings Ltd`.

**Update spreadsheet** writes straight into the file instead. New files are
added at the bottom and existing rows are refreshed. Deleted files are
flagged if you mapped a Present/Missing column. A backup goes into
`_tracker_backups` first.

Your setup is remembered between runs. Use **Save setup…** and
**Load setup…** to keep one setup per project.

### Good to know

- **Sizes** can follow Finder (1 KB = 1,000 bytes, the default on a Mac) or
  Windows (1 KB = 1,024 bytes, the default on Windows), set under Options, so
  the numbers match what you see on your computer.
- Mac items that Finder shows as single files but are really folders (apps,
  Final Cut Pro / Photos / Logic libraries and similar) are listed as one
  item, sized by their whole contents.
- Shortcuts (symlinks) to files are sized by the file they point to.

- Close the spreadsheet in Excel before clicking **Update spreadsheet**.
- New rows copy the formatting of the first data row. Excel tables and
  filters are extended to cover them. Formulas and other sheets are kept.
- Old `.xls` and Apple Numbers files must be saved or exported as `.xlsx`
  first.
- On a Mac, the first time you scan Documents, Desktop or Downloads, macOS
  asks for permission. Click **Allow**.

## Making a new release

Builds run automatically on GitHub every time you push to `main`:

1. Change `version` in `Cargo.toml` (e.g. `1.0.0` → `1.0.1`).
2. Commit and push to `main`.

About 10 minutes later, **File Tracker v1.0.1** appears on the Releases page
with fresh downloads. If you push without changing the version, the current
release's downloads are rebuilt in place. Each run's builds are also
available on the **Actions** tab, under **Artifacts**.

## Building it yourself

You need [Rust](https://rustup.rs).

- **Mac:** `bash build_mac.sh` creates `dist/File Tracker.app`.
- **Windows:** double-click `build_windows.bat` to create
  `dist\File Tracker.exe`.
- **Tests:** `pip install openpyxl`, then
  `python tests/make_fixture.py /tmp/ft`, then
  `FT_SANDBOX=/tmp/ft cargo test`.

## What's in here

| Path | What it is |
|---|---|
| `src/main.rs` | The app window (built with [egui](https://github.com/emilk/egui)) |
| `src/core.rs` | Folder scanning, column values, spreadsheet reading and writing, tests |
| `src/extract.rs` | Custom column rules (pieces of folder and file names), tests |
| `src/builder.rs` | The column builder window |
| `src/sorting.rs` | Sorting the grid (natural order, renumbering) |
| `src/update.rs` | In-app updates from the Releases page |
| `.github/workflows/build.yml` | Builds the Mac and Windows apps and publishes releases |
| `python/` | The original Python version (needs Python 3; see `python/README.txt`) |
| `tests/make_fixture.py` | Creates a sample folder and spreadsheets for the tests |
