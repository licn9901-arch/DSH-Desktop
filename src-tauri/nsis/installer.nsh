; Extra NSIS hooks for DeepSeek Harness Desktop.
;
; Tauri's default NSIS installer already registers an uninstaller in
; "Settings > Apps" and writes uninstall.exe to the install directory.
; These hooks additionally create an "Uninstall DeepSeek Harness Desktop"
; shortcut in the Start Menu folder next to the app shortcut, then clean
; it up when the app is uninstalled.

!macro NSIS_HOOK_POSTINSTALL
  ${If} $AppStartMenuFolder != ""
    CreateShortCut "$SMPROGRAMS\$AppStartMenuFolder\Uninstall DeepSeek Harness Desktop.lnk" "$INSTDIR\uninstall.exe"
  ${Else}
    CreateShortCut "$SMPROGRAMS\Uninstall DeepSeek Harness Desktop.lnk" "$INSTDIR\uninstall.exe"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; The standard uninstall section resolves $AppStartMenuFolder and removes
  ; the app shortcut before this hook runs. Remove the extra uninstall
  ; shortcut now and retry RMDir so the folder is actually removed.
  ${If} $AppStartMenuFolder != ""
    Delete "$SMPROGRAMS\$AppStartMenuFolder\Uninstall DeepSeek Harness Desktop.lnk"
    RMDir "$SMPROGRAMS\$AppStartMenuFolder"
  ${Else}
    Delete "$SMPROGRAMS\Uninstall DeepSeek Harness Desktop.lnk"
  ${EndIf}

  ; The desktop shell writes its own log to %LOCALAPPDATA%\dsh-desktop.
  ; Honor the uninstaller's "delete app data" checkbox for that directory.
  ${If} $DeleteAppDataCheckboxState = 1
    SetShellVarContext current
    RmDir /r "$LOCALAPPDATA\dsh-desktop"
  ${EndIf}

  ; Cleanup failures here (e.g. RMDir on a non-empty shared folder) should
  ; not change the uninstaller's success status.
  ClearErrors
!macroend
