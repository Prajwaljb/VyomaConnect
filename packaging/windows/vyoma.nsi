; Vyoma's installer for Windows (NSIS 3, zlib licence): for this user only, so no administrator.
; Puts vyoma.exe (the window) and vyoma-windows.exe (the agent) in %LOCALAPPDATA%\Programs\Vyoma, a
; Start menu entry, and an entry in Settings > Apps to uninstall it. Opening Vyoma at the end starts
; the agent and has it start with Windows (vyoma.exe runs `vyoma-windows.exe --install`).
; A running Vyoma is closed first (`--quit`), so an update replaces it in place. Uninstalling stops
; it, takes out its start-with-Windows and Send to entries and toast registration (`--uninstall`) and the programs;
; pairings stay in %LOCALAPPDATA%\Vyoma, as they do when an app is removed and put back, unless the
; person asks for them to go. Firewall rules made with Let phones through need an administrator to
; remove, so they stay (they only let in that program, which is gone).
;
; makensis /DVERSION=0.12.0 /DSRC=<folder with both programs and LICENSE> /DOUT=<setup.exe> [/DSIGN=<cmd>] vyoma.nsi
; SIGN: a command that signs the file it's given (CI's sign.cmd), run on the uninstaller and the installer.
; Built on GitHub's Windows runners (.github/workflows/windows.yml); not yet run on a real Windows PC.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!ifndef VERSION
  !error "makensis /DVERSION=x.y.z"
!endif
!ifndef SRC
  !error "makensis /DSRC=<folder with vyoma.exe, vyoma-windows.exe, LICENSE>"
!endif
!ifndef OUT
  !define OUT "Vyoma-setup-${VERSION}.exe"
!endif

!ifdef SIGN
  ; NSIS 3.08 or newer.
  !finalize '"${SIGN}" "%1"' = 0
  !uninstfinalize '"${SIGN}" "%1"' = 0
!endif

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vyoma"

Name "Vyoma"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\Vyoma"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
BrandingText "Vyoma ${VERSION}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Vyoma"
VIAddVersionKey "FileDescription" "Vyoma setup"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "GPL-3.0-or-later"

!include "MUI2.nsh"

!define MUI_ICON "..\..\desktop\windows\dist\vyoma.ico"
!define MUI_UNICON "..\..\desktop\windows\dist\vyoma.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "Vyoma ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "Vyoma connects this PC with Vyoma on your Android phone: files, links and the clipboard both ways, your phone's notifications and texts here, its touchpad and keyboard for this PC, music controls, and this PC's screen on the phone.$\r$\n$\r$\nIt installs for you only and needs no administrator."
!define MUI_FINISHPAGE_RUN "$INSTDIR\vyoma.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open Vyoma"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${SRC}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Vyoma"
  SetShellVarContext current
  ; A running Vyoma lets go of its files first.
  IfFileExists "$INSTDIR\vyoma-windows.exe" 0 +2
    ExecWait '"$INSTDIR\vyoma-windows.exe" --quit'
  SetOutPath "$INSTDIR"
  File "${SRC}\vyoma.exe"
  File "${SRC}\vyoma-windows.exe"
  File /oname=LICENSE.txt "${SRC}\LICENSE"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Vyoma.lnk" "$INSTDIR\vyoma.exe" "" "$INSTDIR\vyoma.exe" 0

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Vyoma"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Vyoma"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://prajwaljb.github.io/VyomaConnect/"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\vyoma.exe,0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  SectionGetSize 0 $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" $0
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  ExecWait '"$INSTDIR\vyoma-windows.exe" --quit'
  ExecWait '"$INSTDIR\vyoma-windows.exe" --uninstall'
  Delete "$INSTDIR\vyoma.exe"
  Delete "$INSTDIR\vyoma-windows.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Vyoma.lnk"
  ; The agent's --uninstall takes these out too; here in case it couldn't run.
  Delete "$SENDTO\Phone (Vyoma).lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  ; Pairings and this PC's identity: kept unless asked (silent uninstalls keep them).
  IfSilent done
  MessageBox MB_YESNO|MB_ICONQUESTION "Also forget the phones paired with this PC?$\r$\n$\r$\nChoose No to keep them for when Vyoma is installed again." /SD IDNO IDNO done
    RMDir /r "$LOCALAPPDATA\Vyoma"
  done:
SectionEnd
