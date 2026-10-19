@echo off
rem Launch the hexe desktop app in dev mode (Tauri + Vite).
rem Usage: dev.bat [scene.hip|scene.hipnc|scene.hexe]   (optional: opens that scene on start via HEXE_OPEN)
rem Uses bun when it is on PATH (tauri.conf.json's beforeDevCommand), else falls back to npm.
setlocal
cd /d "%~dp0"
where npm >nul 2>nul || (echo npm not found on PATH & exit /b 1)
if not exist node_modules (
  echo Installing UI dependencies...
  call npm install || exit /b 1
)
if not "%~1"=="" set "HEXE_OPEN=%~f1"
where bun >nul 2>nul && goto bun
echo bun not found: running the UI dev server with npm instead
call npx tauri dev --config "{\"build\":{\"beforeDevCommand\":\"npm run dev\"}}"
goto end
:bun
call npx tauri dev
:end
endlocal
