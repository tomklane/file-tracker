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

Settings are on the left and the grid is on the right. The grid updates by
itself whenever you change a setting or add, rename or delete files in the
folder.

1. **Folder to scan.** Choose the top folder. You can limit it to file types
   (`pdf, docx`) or skip names (`*.tmp, Archive, Old*`).
   **Each row is** a file, **a folder at a level** (e.g. level 1 = every
   folder directly inside, such as each shot or project folder), or **each
   folder with files in it** (e.g. every image-sequence folder, at whatever
   depth). Folder rows are sized by adding up everything inside them, and
   "Date modified" is the newest file inside.
2. **Tracking spreadsheet.** Choose your existing tracker. The app finds the
   header row itself, even below a title, and you can change it. Or choose
   **Start from new columns** and type the column names you want.
3. **What goes in each column.** Choose from: file name, folder, path,
   clickable link, size (automatic units, or exactly in KB, MB, GB or TB),
   number of files, dates, a running ID (`DOC-###`), Present/Missing,
   fixed text, or **Custom** (below). Columns left on **leave as is** stay
   blank in the grid and are never touched in your sheet.
4. **Copy from the grid.** The grid shows your sheet's column letters and
   the actual row numbers the files will land on. By default it lists only
   files that aren't in your sheet yet.
   - Drag across cells to select them. Shift-click extends the selection.
     Click a column letter or a row number to take a whole column or row.
     Ctrl/⌘+A selects everything.
   - Press Ctrl+C or ⌘C (or click **Copy selection** or **Copy all rows**).
   - The app tells you which cell to click in your sheet (e.g. "Paste at
     cell A12"). Click it and paste.
   - Tick **Include header row** to copy the column names too.
   - **Sort** by any column with the small arrows on its heading: click once
     for A→Z, again for Z→A, and a third time to go back to the order the
     files were found in. Numbers inside text sort properly (`Rev2` before
     `Rev10`), and empty cells always go last. Running numbers (`DOC-###`)
     renumber to follow the sorted order, so what you paste stays in
     sequence. The sort is remembered between runs.

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
