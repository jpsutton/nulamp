; nulamp installer for Windows (NSIS 3, Modern UI 2).
;
; makensis /DVERSION=<x.y.z> /DSRCDIR=<folder with nulamp.exe> /DOUTFILE=<installer.exe> nulamp.nsi
;
; Installs per user by default (no administrator prompt); machine-wide when chosen and allowed.
; File associations: .wsz skins open with nulamp; audio files and playlists are registered as
; "Open with" choices and in Default Apps, without taking over existing defaults.

Unicode true
SetCompressor /SOLID lzma

!ifndef VERSION
  !error "VERSION is not defined"
!endif
!ifndef SRCDIR
  !error "SRCDIR is not defined"
!endif
!ifndef OUTFILE
  !define OUTFILE "nulamp-${VERSION}-setup.exe"
!endif

!define APPNAME "nulamp"
!define PUBLISHER "nulamp contributors"
!define WEBSITE "https://github.com/jpsutton/nulamp"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\nulamp"
!define APPKEY "Software\nulamp"
!define ICON "..\..\assets\windows\nulamp.ico"

Name "${APPNAME} ${VERSION}"
OutFile "${OUTFILE}"
BrandingText "${APPNAME} ${VERSION}"

; Per-user or machine-wide installs (MultiUser.nsh ships with NSIS).
!define MULTIUSER_EXECUTIONLEVEL Highest
!define MULTIUSER_MUI
!define MULTIUSER_INSTALLMODE_COMMANDLINE
!define MULTIUSER_INSTALLMODE_DEFAULT_CURRENTUSER
!define MULTIUSER_INSTALLMODE_INSTDIR "${APPNAME}"
!define MULTIUSER_INSTALLMODE_INSTDIR_REGISTRY_KEY "${APPKEY}"
!define MULTIUSER_INSTALLMODE_INSTDIR_REGISTRY_VALUENAME "InstallDir"
!define MULTIUSER_USE_PROGRAMFILES64
!include MultiUser.nsh
!include MUI2.nsh
!include LogicLib.nsh

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${APPNAME} installer"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "MIT OR Apache-2.0"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC
!define MUI_FINISHPAGE_RUN "$INSTDIR\nulamp.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APPNAME}"
!define MUI_FINISHPAGE_SHOWREADME "$INSTDIR\README.md"
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Show the README"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "license.txt"
!insertmacro MULTIUSER_PAGE_INSTALLMODE
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; Audio and playlist types offered as "Open with" choices.
!define AUDIO_EXTS "mp3 mp2 mp1 ogg oga opus flac wav aif aiff m4a mp4 aac alac wma wv ape mpc tta mid midi rmi kar mod s3m xm it mptm"
!define PLAYLIST_EXTS "m3u m3u8 pls xspf asx wpl b4s"

; Registers "<ext>" with ProgID $R1 as an "Open with" choice and in nulamp's Default Apps
; capabilities. $R0 = extension without the dot.
Function RegisterOpenWith
  WriteRegStr SHCTX "Software\Classes\.$R0\OpenWithProgids" "$R1" ""
  WriteRegStr SHCTX "${APPKEY}\Capabilities\FileAssociations" ".$R0" "$R1"
FunctionEnd

Function un.UnregisterOpenWith
  DeleteRegValue SHCTX "Software\Classes\.$R0\OpenWithProgids" "$R1"
  DeleteRegKey /ifempty SHCTX "Software\Classes\.$R0\OpenWithProgids"
  DeleteRegKey /ifempty SHCTX "Software\Classes\.$R0"
FunctionEnd

; Calls Function $R2 for each word of the space-separated list $R3, with the word in $R0.
!macro ForEachExt LIST FUNC PROGID
  Push $R3
  StrCpy $R3 "${LIST} "
  StrCpy $R1 "${PROGID}"
  ${Do}
    StrCpy $R5 0
    ${Do}
      StrCpy $R4 $R3 1 $R5
      ${IfThen} $R4 == " " ${|} ${ExitDo} ${|}
      IntOp $R5 $R5 + 1
    ${Loop}
    StrCpy $R0 $R3 $R5
    IntOp $R5 $R5 + 1
    StrCpy $R3 $R3 "" $R5
    Call ${FUNC}
  ${LoopUntil} $R3 == ""
  Pop $R3
!macroend

!macro WriteProgId PROGID DESC
  WriteRegStr SHCTX "Software\Classes\${PROGID}" "" "${DESC}"
  WriteRegStr SHCTX "Software\Classes\${PROGID}\DefaultIcon" "" "$INSTDIR\nulamp.ico"
  WriteRegStr SHCTX "Software\Classes\${PROGID}\shell\open\command" "" '"$INSTDIR\nulamp.exe" "%1"'
!macroend

Section "nulamp (required)" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${SRCDIR}\nulamp.exe"
  File "${ICON}"
  File "${SRCDIR}\README.md"
  File "${SRCDIR}\LICENSE-MIT"
  File "${SRCDIR}\LICENSE-APACHE"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr SHCTX "${APPKEY}" "InstallDir" "$INSTDIR"
  ; The application registration Windows uses for "Open with" and Default Apps.
  WriteRegStr SHCTX "Software\Classes\Applications\nulamp.exe\shell\open\command" "" '"$INSTDIR\nulamp.exe" "%1"'
  WriteRegStr SHCTX "Software\Classes\Applications\nulamp.exe" "FriendlyAppName" "${APPNAME}"
  WriteRegStr SHCTX "${APPKEY}\Capabilities" "ApplicationName" "${APPNAME}"
  WriteRegStr SHCTX "${APPKEY}\Capabilities" "ApplicationDescription" "Audio player for classic Winamp 2.x skins"
  WriteRegStr SHCTX "Software\RegisteredApplications" "${APPNAME}" "${APPKEY}\Capabilities"

  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr SHCTX "${UNINSTKEY}" "URLInfoAbout" "${WEBSITE}"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\nulamp.ico"
  WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe" /$MultiUser.InstallMode'
  WriteRegStr SHCTX "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /$MultiUser.InstallMode /S'
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair" 1
  SectionGetSize ${SecMain} $0
  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" $0
SectionEnd

Section "Start menu shortcut" SecStartMenu
  CreateShortCut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\nulamp.exe" "" "$INSTDIR\nulamp.ico"
SectionEnd

Section /o "Desktop shortcut" SecDesktop
  CreateShortCut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\nulamp.exe" "" "$INSTDIR\nulamp.ico"
SectionEnd

Section "Open .wsz skins with nulamp" SecSkins
  !insertmacro WriteProgId "nulamp.skin" "nulamp skin"
  WriteRegStr SHCTX "Software\Classes\.wsz" "" "nulamp.skin"
  WriteRegStr SHCTX "Software\Classes\.wsz" "Content Type" "application/x-nulamp-skin"
  WriteRegStr SHCTX "${APPKEY}\Capabilities\FileAssociations" ".wsz" "nulamp.skin"
SectionEnd

Section "Offer nulamp for audio files and playlists" SecMedia
  !insertmacro WriteProgId "nulamp.audio" "Audio file"
  !insertmacro WriteProgId "nulamp.playlist" "Playlist"
  !insertmacro ForEachExt "${AUDIO_EXTS}" RegisterOpenWith "nulamp.audio"
  !insertmacro ForEachExt "${PLAYLIST_EXTS}" RegisterOpenWith "nulamp.playlist"
SectionEnd

Section /o "Plugin SDK (header and guide)" SecSdk
  SetOutPath "$INSTDIR\plugin-sdk"
  File "${SRCDIR}\plugin-sdk\nulamp_plugin.h"
  File "${SRCDIR}\plugin-sdk\plugins.md"
SectionEnd

Section "-Notify shell"
  ; SHCNE_ASSOCCHANGED: refresh icons and associations.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "The player."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecStartMenu} "A shortcut in the Start menu."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "A shortcut on the desktop."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecSkins} "Double-clicking a .wsz skin installs and applies it."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMedia} "Lists nulamp under Open with and in Default Apps for audio files and playlists. Your current defaults are not changed."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecSdk} "C header and guide for writing nulamp plugins."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Function .onInit
  !insertmacro MULTIUSER_INIT
FunctionEnd

Function un.onInit
  !insertmacro MULTIUSER_UNINIT
FunctionEnd

Section "Uninstall"
  Delete "$INSTDIR\nulamp.exe"
  Delete "$INSTDIR\nulamp.ico"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\plugin-sdk\nulamp_plugin.h"
  Delete "$INSTDIR\plugin-sdk\plugins.md"
  RMDir "$INSTDIR\plugin-sdk"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  Delete "$DESKTOP\${APPNAME}.lnk"

  ; Associations: only what nulamp registered.
  ReadRegStr $0 SHCTX "Software\Classes\.wsz" ""
  ${If} $0 == "nulamp.skin"
    DeleteRegKey SHCTX "Software\Classes\.wsz"
  ${EndIf}
  !insertmacro ForEachExt "${AUDIO_EXTS}" un.UnregisterOpenWith "nulamp.audio"
  !insertmacro ForEachExt "${PLAYLIST_EXTS}" un.UnregisterOpenWith "nulamp.playlist"
  DeleteRegKey SHCTX "Software\Classes\nulamp.skin"
  DeleteRegKey SHCTX "Software\Classes\nulamp.audio"
  DeleteRegKey SHCTX "Software\Classes\nulamp.playlist"
  DeleteRegKey SHCTX "Software\Classes\Applications\nulamp.exe"
  DeleteRegValue SHCTX "Software\RegisteredApplications" "${APPNAME}"
  DeleteRegKey SHCTX "${APPKEY}"
  DeleteRegKey SHCTX "${UNINSTKEY}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd
