# File Tracker

A desktop app for **macOS and Windows**. It scans a folder (as seen in Finder
or File Explorer) and either:

- shows the files in a window you can **copy and paste** into any
  spreadsheet (Excel, Numbers, Google Sheets), or
- **updates your tracking spreadsheet** (.xlsx, .xlsm, .csv) directly.

It reads the columns your spreadsheet already has, and you choose what goes
in each one. That means it works with any folder layout and any spreadsheet
layout.

## Download

Go to **[Releases](../../releases/latest)** and download:

| Your computer | File |
|---|---|
| Mac (Apple Silicon or Intel) | `File-Tracker-Mac.zip`. Unzip it and drag **File Tracker** to Applications. |
| Windows | `File-Tracker-Windows.exe`. It's a single file with nothing to install. |

**First launch.** The apps aren't signed by Apple or Microsoft, so:
- **Mac:** open the app once. When macOS blocks it, go to **System Settings ›
  Privacy & Security**, scroll down and click **Open Anyway**.
- **Windows:** if SmartScreen appears, click **More info › Run anyway**.

## Using it

1. **Folder to scan.** Choose the top folder. You can limit it to file types
   (`pdf, docx`) or skip names (`*.tmp, Archive, Old*`).
2. **Tracking spreadsheet.** Choose your existing tracker. The app finds the
   header row itself, even below a title, and you can change it. Or choose
   **Create a new one** and type the column names you want.
3. **What goes in each column.** Choose from: file name, part of the file
   name (`PRJ001_Beam-Layout_RevA` gives part 1 = `PRJ001` and
   part -1 = `RevA`), folder at a level, path, clickable link, size, dates,
   a running ID (`DOC-###`), Present/Missing, or fixed text. Columns left on
   **leave as is** are never touched.
4. Click one of the two main buttons:
   - **Show list to copy** opens a window of the files in your sheet's column
     order. Click **Copy all**, or click rows to select them
     (Ctrl/⌘‑click or Shift‑click for several) and press Ctrl+C or ⌘C. Then
     paste into the first empty row of your sheet. Nothing is saved to your
     file. By default, only files not yet in the sheet are listed.
   - **Update spreadsheet** writes straight into the file. New files are
     added at the bottom and existing rows are refreshed. Deleted files are
     flagged if you mapped a Present/Missing column. A backup goes into
     `_tracker_backups` first.

Your setup is remembered between runs. Use **Save setup…** and
**Load setup…** to keep one setup per project.

### Good to know

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
| `.github/workflows/build.yml` | Builds the Mac and Windows apps and publishes releases |
| `python/` | The original Python version (needs Python 3; see `python/README.txt`) |
| `tests/make_fixture.py` | Creates a sample folder and spreadsheets for the tests |
