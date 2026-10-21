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
; The progress bar is the page; the file-by-file log is one click away.
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "devd"
VIAddVersionKey "FileDescription" "devd installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT"

; The standard Windows installer look, with devd's own icon.
!define MUI_ICON "${ROOT}\crates\devd-desktop\icons\icon.ico"
!define MUI_UNICON "${ROOT}\crates\devd-desktop\icons\icon.ico"
!define MUI_ABORTWARNING
BrandingText "devd ${VERSION}"

; Four pages: welcome, where, installing, done. Nothing else to answer.
!define MUI_WELCOMEPAGE_TITLE "Welcome to devd"
!define MUI_WELCOMEPAGE_TEXT "Your sites, services, runtimes and agents in one place.$\r$\n$\r$\nThis takes a minute. It installs for you alone, so no administrator password is needed, and it adds devd to your PATH and the Start menu.$\r$\n$\r$\nPHP, nginx and MariaDB are not bundled: devd downloads them from their publishers the first time it starts, and checks each one before it runs it."
!insertmacro MUI_PAGE_WELCOME

!define MUI_DIRECTORYPAGE_TEXT_TOP "Choose the folder to install devd in. Click Browse to pick another one, then click Install.$\r$\n$\r$\nYour projects, databases and certificates are kept in $LOCALAPPDATA\devd whichever folder you choose, so they survive an upgrade or a reinstall."
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE DirLeave
!insertmacro MUI_PAGE_DIRECTORY

!insertmacro MUI_PAGE_INSTFILES

!define MUI_FINISHPAGE_TITLE "devd is ready"
!define MUI_FINISHPAGE_RUN "$INSTDIR\launch.vbs"
!define MUI_FINISHPAGE_RUN_TEXT "Set up devd and open it now"
!define MUI_FINISHPAGE_TEXT "devd is installed.$\r$\n$\r$\nThe first start sets everything up and shows what it is doing: it checks what your machine needs, downloads PHP, nginx and MariaDB (about 140 MB), starts them and creates your first site. It takes a few minutes, and devd opens when it is done.$\r$\n$\r$\nYou can run devd from the Start menu or the desktop at any time."
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; devd installs for the current user and asks for no administrator password,
; so a folder that needs one cannot be written to. Say so on the page where the
; folder is chosen, not as a failure halfway through copying files.
Function DirLeave
  StrLen $1 "$PROGRAMFILES"
  StrCpy $2 "$INSTDIR" $1
  StrCmp $2 "$PROGRAMFILES" protected 0
  StrLen $1 "$PROGRAMFILES64"
  StrCpy $2 "$INSTDIR" $1
  StrCmp $2 "$PROGRAMFILES64" protected 0
  StrLen $1 "$WINDIR"
  StrCpy $2 "$INSTDIR" $1
  StrCmp $2 "$WINDIR" protected done
  protected:
    MessageBox MB_OK|MB_ICONEXCLAMATION "devd installs for your user without an administrator password, so it cannot write to $INSTDIR.$\r$\n$\r$\nChoose a folder you own, such as D:\devd or the default."
    Abort
  done:
FunctionEnd

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

; A desktop shortcut is part of the install rather than a question: there is no
; components page to ask it on.
Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\devd.lnk" "$INSTDIR\devd-desktop.exe" "" "$INSTDIR\devd-desktop.exe" 0
SectionEnd

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
