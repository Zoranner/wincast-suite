@echo off
setlocal EnableExtensions

set "TASK_NAME=WinCast Host"
set "APP_DIR=%APPDATA%\WinCast"
set "VBS_PATH=%APP_DIR%\run-host-hidden.vbs"
set "CONFIG_PATH=%APP_DIR%\host.toml"
set "ACTION=%~1"

if "%ACTION%"=="" set "ACTION=install"

if /I "%ACTION%"=="install" goto :install
if /I "%ACTION%"=="uninstall" goto :uninstall
if /I "%ACTION%"=="remove" goto :uninstall
if /I "%ACTION%"=="status" goto :status
if /I "%ACTION%"=="run" goto :run

echo Usage:
echo   %~nx0 install [path-to-wincast-host.exe]
echo   %~nx0 uninstall
echo   %~nx0 status
echo   %~nx0 run [path-to-wincast-host.exe]
exit /b 2

:install
call :resolve_host "%~2" || exit /b 1
call :write_launcher || exit /b 1

set "WINCAST_HOST_VBS=%VBS_PATH%"
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference = 'Stop';" ^
  "$action = New-ScheduledTaskAction -Execute 'wscript.exe' -Argument ('//B //Nologo ""' + $env:WINCAST_HOST_VBS + '""');" ^
  "$trigger = New-ScheduledTaskTrigger -AtLogOn;" ^
  "$settings = New-ScheduledTaskSettingsSet -Hidden -AllowStartIfOnBatteries -DisallowStartIfOnBatteries:$false -ExecutionTimeLimit (New-TimeSpan -Seconds 0);" ^
  "Register-ScheduledTask -TaskName '%TASK_NAME%' -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null"

if errorlevel 1 (
    echo Failed to register startup task.
    exit /b 1
)

echo Installed startup task: %TASK_NAME%
echo Host executable: %HOST_EXE%
if not exist "%CONFIG_PATH%" (
    echo Warning: host config not found: %CONFIG_PATH%
)
exit /b 0

:uninstall
schtasks.exe /Delete /TN "%TASK_NAME%" /F >nul 2>nul
if exist "%VBS_PATH%" del /F /Q "%VBS_PATH%" >nul 2>nul
echo Removed startup task: %TASK_NAME%
exit /b 0

:status
schtasks.exe /Query /TN "%TASK_NAME%" /FO LIST /V
exit /b %ERRORLEVEL%

:run
call :resolve_host "%~2" || exit /b 1
call :write_launcher || exit /b 1
wscript.exe //B //Nologo "%VBS_PATH%"
exit /b %ERRORLEVEL%

:resolve_host
set "HOST_EXE=%~1"
if not "%HOST_EXE%"=="" goto :host_from_arg

if exist "%~dp0wincast-host.exe" (
    set "HOST_EXE=%~dp0wincast-host.exe"
    goto :host_found
)

if exist "%~dp0..\..\target\release\wincast-host.exe" (
    set "HOST_EXE=%~dp0..\..\target\release\wincast-host.exe"
    goto :host_found
)

for %%I in (wincast-host.exe) do (
    if not "%%~$PATH:I"=="" (
        set "HOST_EXE=%%~$PATH:I"
        goto :host_found
    )
)

echo Cannot find wincast-host.exe.
echo Pass the full path explicitly:
echo   %~nx0 install C:\path\to\wincast-host.exe
exit /b 1

:host_from_arg
if not exist "%HOST_EXE%" (
    echo Host executable not found: %HOST_EXE%
    exit /b 1
)

:host_found
for %%I in ("%HOST_EXE%") do (
    set "HOST_EXE=%%~fI"
    set "HOST_DIR=%%~dpI"
)
exit /b 0

:write_launcher
if not exist "%APP_DIR%" mkdir "%APP_DIR%" >nul 2>nul
if errorlevel 1 (
    echo Failed to create directory: %APP_DIR%
    exit /b 1
)

> "%VBS_PATH%" echo Set shell = CreateObject("WScript.Shell")
>> "%VBS_PATH%" echo shell.CurrentDirectory = "%HOST_DIR%"
>> "%VBS_PATH%" echo shell.Run Chr(34) ^& "%HOST_EXE%" ^& Chr(34), 0, False

if errorlevel 1 (
    echo Failed to write launcher: %VBS_PATH%
    exit /b 1
)
exit /b 0
