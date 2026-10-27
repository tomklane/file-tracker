@echo off
rem Builds "File Tracker.exe" into the dist folder. Double-click to run.
cd /d "%~dp0"

where cargo >nul 2>nul
if not %errorlevel%==0 (
    echo.
    echo  Rust isn't installed yet.
    echo  1. Download and run rustup-init.exe from https://rustup.rs  ^(opening now^)
    echo     Accept the offer to install the Visual Studio build tools.
    echo  2. Then double-click build_windows.bat again.
    echo.
    start "" "https://rustup.rs"
    pause
    exit /b 1
)

echo Building File Tracker (the first build takes a few minutes)...
cargo build --release
if not %errorlevel%==0 (
    echo Build failed - see the messages above.
    pause
    exit /b 1
)
if not exist dist mkdir dist
copy /y "target\release\file_tracker.exe" "dist\File Tracker.exe" >nul
echo.
echo Done: %cd%\dist\File Tracker.exe
echo You can move it anywhere, e.g. your Desktop. It doesn't need anything else to run.
pause
