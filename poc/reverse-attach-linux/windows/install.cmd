@echo off
rem Double-click to install the OpenAB Windows hands node (POC) for the current user.
rem No admin needed. Runs install-winpoc.ps1 beside it; pass -AllowLogin you@example.com
rem to use Tailscale identity instead of a generated bearer token.
setlocal
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-winpoc.ps1" -Install %*
echo.
if errorlevel 1 ( echo Install failed - see the message above. ) else ( echo Done. The node auto-starts at every logon and is running now. )
echo.
pause
