@echo off
rem Double-click to start File Tracker on Windows.
cd /d "%~dp0"

where pyw >nul 2>nul
if %errorlevel%==0 (
    start "" pyw "file_tracker.py"
    exit /b
)
where pythonw >nul 2>nul
if %errorlevel%==0 (
    start "" pythonw "file_tracker.py"
    exit /b
)

echo.
echo  File Tracker needs Python 3.
echo.
echo  1. Download it from python.org (opening now).
echo  2. Run the installer and tick "Add python.exe to PATH".
echo  3. Double-click "Start File Tracker (Windows)" again.
echo.
start "" "https://www.python.org/downloads/windows/"
pause
