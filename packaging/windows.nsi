; OpenScreenRec Windows installer (NSIS).
; Built by the release workflow from the repo root:
;   makensis /DAPP_VERSION=x.y.z /DEXE_PATH=target\release\openscreenrec.exe
;            /DLICENSE_PATH=LICENSE /DREADME_PATH=README.md /DOUT_DIR=dist
;            packaging\windows.nsi

Unicode true
Name "OpenScreenRec ${APP_VERSION}"
OutFile "${OUT_DIR}\openscreenrec-setup.exe"
InstallDir "$PROGRAMFILES64\OpenScreenRec"
RequestExecutionLevel admin

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Section "Install"
  SetOutPath $INSTDIR
  File /oname=openscreenrec.exe "${EXE_PATH}"
  File /oname=LICENSE.txt "${LICENSE_PATH}"
  File /oname=README.md "${README_PATH}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateDirectory "$SMPROGRAMS\OpenScreenRec"
  CreateShortcut "$SMPROGRAMS\OpenScreenRec\OpenScreenRec.lnk" "$INSTDIR\openscreenrec.exe"
  CreateShortcut "$DESKTOP\OpenScreenRec.lnk" "$INSTDIR\openscreenrec.exe"

  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenScreenRec" "DisplayName" "OpenScreenRec"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenScreenRec" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenScreenRec" "UninstallString" "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\openscreenrec.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$DESKTOP\OpenScreenRec.lnk"
  RMDir "$SMPROGRAMS\OpenScreenRec"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenScreenRec"
SectionEnd
