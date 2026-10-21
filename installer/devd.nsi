; devd installer.
;
; Per-user install: no administrator prompt, everything under
; %LOCALAPPDATA%\Programs\devd, shortcuts and an uninstall entry in HKCU.
;
; Built by scripts/release-installer.ps1, which passes VERSION, SOURCE and
; OUTFILE on the makensis command line.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef SOURCE
  !define SOURCE "target\release\devd.exe"
!endif
!ifndef DESKTOP
  !define DESKTOP "target\release\devd-desktop.exe"
!endif
!ifndef WEBVIEW2
  !define WEBVIEW2 "target\release\WebView2Loader.dll"
!endif
!ifndef ROOT
  !define ROOT ".."
!endif
!ifndef OUTFILE
  !define OUTFILE "dist\devd-setup.exe"
!endif

!define APPNAME "devd"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\devd"

Name "${APPNAME} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\devd"
InstallDirRegKey HKCU "Software\devd" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "devd"
VIAddVersionKey "FileDescription" "devd installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT"

!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "devd ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "devd is a local development environment: PHP, Node.js, Go, nginx, MariaDB and HTTPS sites, with one permissioned interface for people and agents.$\r$\n$\r$\nThis installs for the current user only. No administrator password is required.$\r$\n$\r$\nPHP, nginx and MariaDB are downloaded from their vendors on first start, never bundled."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES

!define MUI_FINISHPAGE_RUN "$INSTDIR\launch.vbs"
!define MUI_FINISHPAGE_RUN_TEXT "Set up devd and open it"
!define MUI_FINISHPAGE_TEXT "devd is installed.$\r$\n$\r$\nThe first start runs the setup, which reports what it is doing as it goes:$\r$\n$\r$\n  1. Requirements - what is missing, and what this run will do about it$\r$\n  2. Runtime - PHP is downloaded and verified by running it$\r$\n  3. Services - nginx and MariaDB are downloaded from their vendors$\r$\n  4. Starting the stack - each service is started and checked$\r$\n  5. First site - a site is created and a certificate is issued$\r$\n  6. Certificate authority - trusted for this user$\r$\n$\r$\nThe downloads are about 140 MB and can take a few minutes. Progress appears in a console window; the application opens when the setup finishes."
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ; Refuse to install over a running copy: its files are locked and its
  ; services would keep the old binaries. Failures here are expected on a
  ; fresh install.
  IfFileExists "$INSTDIR\devd.exe" 0 done
    nsExec::ExecToLog '"$INSTDIR\devd.exe" stack down'
    nsExec::ExecToLog 'taskkill /F /IM devd.exe'
    nsExec::ExecToLog 'taskkill /F /IM devd-desktop.exe'
    Sleep 800
  done:

  ; The devd window is drawn by the Microsoft Edge WebView2 runtime. It ships
  ; with current Edge, but not on every machine, and without it the window
  ; cannot open at all. Ask before installing it.
  ClearErrors
  ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  IfErrors 0 after_webview2
  ClearErrors
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  IfErrors 0 after_webview2
  ClearErrors
  ReadRegStr $0 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  IfErrors 0 after_webview2

  IfSilent silent_webview2
  MessageBox MB_YESNO|MB_ICONQUESTION "The devd window needs the Microsoft Edge WebView2 runtime, and it is not installed on this machine.$\r$\n$\r$\nInstall it now? It is a small download from Microsoft." IDNO skip_webview2
  DetailPrint "Installing the WebView2 runtime..."
  nsExec::ExecToLog 'curl.exe -L -o "$TEMP\MicrosoftEdgeWebview2Setup.exe" "https://go.microsoft.com/fwlink/p/?LinkId=2124703"'
  nsExec::ExecToLog '"$TEMP\MicrosoftEdgeWebview2Setup.exe" /silent /install'
  Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
  Goto after_webview2
  silent_webview2:
  skip_webview2:
  after_webview2:
FunctionEnd

Section "devd (required)" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${SOURCE}"
  File "${DESKTOP}"
  File "${WEBVIEW2}"
  File "${ROOT}\LICENSE"
  File "${ROOT}\README.md"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Launcher: set up on first run, then open the application without a console.
  ; `app` is expanded here, at install time, so the VBScript contains a literal
  ; path and no nested quoting.
  FileOpen $9 "$INSTDIR\launch.vbs" w
  FileWrite $9 'Dim fso, sh, app, home, code$\r$\n'
  FileWrite $9 'Set fso = CreateObject("Scripting.FileSystemObject")$\r$\n'
  FileWrite $9 'Set sh = CreateObject("WScript.Shell")$\r$\n'
  FileWrite $9 'app = "$INSTDIR"$\r$\n'
  FileWrite $9 'home = sh.ExpandEnvironmentStrings("%LOCALAPPDATA%") & "\devd"$\r$\n'
  FileWrite $9 'If Not fso.FileExists(home & "\services\nginx\nginx.exe") Then$\r$\n'
  FileWrite $9 '  code = sh.Run("""" & app & "\devd.exe"" setup --yes", 1, True)$\r$\n'
  FileWrite $9 '  If code <> 0 Then$\r$\n'
  FileWrite $9 '    MsgBox "devd could not finish setting up." & vbCrLf & vbCrLf & "Open PowerShell and run:" & vbCrLf & "  """ & app & "\devd.exe"" setup", 16, "devd"$\r$\n'
  FileWrite $9 '    WScript.Quit 1$\r$\n'
  FileWrite $9 '  End If$\r$\n'
  FileWrite $9 'End If$\r$\n'
  FileWrite $9 'Call sh.Run("""" & app & "\devd-desktop.exe""", 0, False)$\r$\n'
  FileClose $9

  ; PATH helpers, so `devd` works in a new terminal.
  FileOpen $9 "$INSTDIR\add-to-path.ps1" w
  FileWrite $9 '$$dir = Split-Path -Parent $$MyInvocation.MyCommand.Path$\r$\n'
  FileWrite $9 '$$p = [Environment]::GetEnvironmentVariable("Path", "User")$\r$\n'
  FileWrite $9 '$$parts = @($$p -split ";" | Where-Object { $$_ -ne "" })$\r$\n'
  FileWrite $9 'if ($$parts -notcontains $$dir) { [Environment]::SetEnvironmentVariable("Path", ((@($$parts) + $$dir) -join ";"), "User") }$\r$\n'
  FileClose $9

  FileOpen $9 "$INSTDIR\remove-from-path.ps1" w
  FileWrite $9 '$$dir = Split-Path -Parent $$MyInvocation.MyCommand.Path$\r$\n'
  FileWrite $9 '$$p = [Environment]::GetEnvironmentVariable("Path", "User")$\r$\n'
  FileWrite $9 '$$parts = @($$p -split ";" | Where-Object { $$_ -ne "" -and $$_ -ne $$dir })$\r$\n'
  FileWrite $9 '[Environment]::SetEnvironmentVariable("Path", ($$parts -join ";"), "User")$\r$\n'
  FileClose $9

  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\add-to-path.ps1"'

  CreateDirectory "$SMPROGRAMS\devd"
  CreateShortcut "$SMPROGRAMS\devd\devd.lnk" "$INSTDIR\devd-desktop.exe" "" "$INSTDIR\devd-desktop.exe" 0
  ; The terminal shortcut runs cmd.exe, so it is given the devd icon: every
  ; entry in the Start menu should carry the same mark.
  CreateShortcut "$SMPROGRAMS\devd\devd terminal.lnk" "$SYSDIR\cmd.exe" '/k ""$INSTDIR\devd.exe" doctor"' "$INSTDIR\devd.exe" 0
  CreateShortcut "$SMPROGRAMS\devd\Uninstall devd.lnk" "$INSTDIR\uninstall.exe" "" "$INSTDIR\uninstall.exe" 0

  WriteRegStr HKCU "Software\devd" "InstallDir" "$INSTDIR"
  ; Whether the WebView2 runtime was found. The application can still be used
  ; from the terminal without it, and the uninstaller says so.
  WriteRegStr HKCU "Software\devd" "WebView2" "$0"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "devd"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "ianclemence"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\devd-desktop.exe"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  ; Repair is left available: re-running the installer over a broken install is
  ; the cheapest way to fix one, and the installer stops the running copy
  ; before it writes.
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 0
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" 12000
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\devd.lnk" "$INSTDIR\devd-desktop.exe" "" "$INSTDIR\devd-desktop.exe" 0
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "devd, its uninstaller and the Start Menu entries."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "A shortcut on the desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  ; Stop the application and the services it started.
  nsExec::ExecToLog '"$INSTDIR\devd.exe" stack down'
  nsExec::ExecToLog 'taskkill /F /IM devd.exe'
  nsExec::ExecToLog 'taskkill /F /IM devd-desktop.exe'
  Sleep 500

  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\remove-from-path.ps1"'

  Delete "$SMPROGRAMS\devd\devd.lnk"
  Delete "$SMPROGRAMS\devd\devd terminal.lnk"
  Delete "$SMPROGRAMS\devd\Uninstall devd.lnk"
  RMDir "$SMPROGRAMS\devd"
  Delete "$DESKTOP\devd.lnk"

  Delete "$INSTDIR\devd.exe"
  Delete "$INSTDIR\devd-desktop.exe"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\launch.vbs"
  Delete "$INSTDIR\add-to-path.ps1"
  Delete "$INSTDIR\remove-from-path.ps1"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "Software\devd"

  ; The data is the person's: their projects, their databases, their
  ; certificates and their settings. Removing devd must not remove those
  ; unless they say so.
  ;
  ; A silent uninstall asks nobody, so it keeps the data. That is the only
  ; safe default for an unattended removal, and it is the same answer the
  ; question gives when it is dismissed without one.
  DetailPrint "Your data is in $LOCALAPPDATA\devd"
  ${IfNot} ${Silent}
    MessageBox MB_YESNO|MB_ICONQUESTION "devd has kept your data in$\r$\n$LOCALAPPDATA\devd$\r$\n$\r$\nIt holds your projects, your databases, your certificates and your settings.$\r$\n$\r$\nKeep it?$\r$\n$\r$\nYes, keep my data (the default).$\r$\nNo, remove it as well." IDNO remove_data
  ${EndIf}
  DetailPrint "Keeping your data in $LOCALAPPDATA\devd"
  Goto after_data
  remove_data:
  DetailPrint "Removing devd's data from $LOCALAPPDATA\devd ..."
  RMDir /r "$LOCALAPPDATA\devd"
  after_data:
SectionEnd
