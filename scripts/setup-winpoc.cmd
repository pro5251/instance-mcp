@echo off
rem Double-click to install (or update) the OpenAB Windows hands node (POC) for the current
rem user. No admin. Downloads the newest release with the Windows ZIP, checks its sha256,
rem installs it, sets up browser tools when Node.js is present and registers it with the
rem local MCP clients it finds (Claude Code, Codex, Copilot CLI, VS Code, Gemini). Arguments
rem go to setup-winpoc.ps1 (see docs\windows-setup.md).
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup-winpoc.ps1" %*
set rc=%errorlevel%
echo.
if %rc% neq 0 ( echo Setup failed - see the message above. ) else ( echo Done. )
echo.
pause
exit /b %rc%
