@echo off
rem Double-click to remove the node. Add -Purge after it in a shell to also delete the
rem token and saved grants.
setlocal
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-winpoc.ps1" -Uninstall %*
echo.
pause
