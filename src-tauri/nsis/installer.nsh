; Extra NSIS hooks for DeepSeek Harness Desktop.
;
; Tauri's default NSIS installer already registers an uninstaller in
; "Settings > Apps" and writes uninstall.exe to the install directory.
; These hooks additionally create an "Uninstall DeepSeek Harness Desktop"
; shortcut in the Start Menu folder next to the app shortcut, then clean
; it up when the app is uninstalled.

!include "StrFunc.nsh"
!include "FileFunc.nsh"
${StrTrimNewLines}

!macro NSIS_HOOK_POSTINSTALL
  ${If} $AppStartMenuFolder != ""
    CreateShortCut "$SMPROGRAMS\$AppStartMenuFolder\Uninstall DeepSeek Harness Desktop.lnk" "$INSTDIR\uninstall.exe"
  ${Else}
    CreateShortCut "$SMPROGRAMS\Uninstall DeepSeek Harness Desktop.lnk" "$INSTDIR\uninstall.exe"
  ${EndIf}

  ; 构建期摘要对应一个不可变插件 store。安装时预置，避免首次启动复制完整插件树。
  ; 正式安装固定使用用户 profile；`/DSHHOME=` 只供隔离安装器 smoke 注入临时目录。
  StrCpy $R0 "$PROFILE\.dsh"
  ${GetParameters} $R6
  ClearErrors
  ${GetOptions} $R6 "/DSHHOME=" $R7
  ${IfNot} ${Errors}
    ${If} $R7 != ""
      StrCpy $R0 $R7
    ${EndIf}
  ${EndIf}
  FileOpen $R1 "$INSTDIR\plugins\store.digest" r
  ${IfNot} ${Errors}
    FileRead $R1 $R2
    FileClose $R1
    ${StrTrimNewLines} $R2 $R2
    ${If} $R2 != ""
      StrCpy $R3 "$R0\profiles\node_modules\.dsh-desktop"
      StrCpy $R4 "$R3\$R2"
      StrCpy $R5 "$R3\.staging-$R2"
      IfFileExists "$R4\plugins.lock.json" 0 preseed_plugins
      IfFileExists "$R4\node_modules\@dsh-desktop\runtime-services\lib\index.js" plugins_preseeded preseed_plugins

      preseed_plugins:
      RmDir /r "$R5"
      RmDir /r "$R4"
      CreateDirectory "$R5\node_modules"
      CopyFiles /SILENT "$INSTDIR\plugins\plugins.lock.json" "$R5"
      CopyFiles /SILENT "$INSTDIR\plugins\node_modules\*.*" "$R5\node_modules"
      IfFileExists "$R5\plugins.lock.json" 0 plugins_preseed_failed
      IfFileExists "$R5\node_modules\@dsh-desktop\runtime-services\lib\index.js" 0 plugins_preseed_failed
      Rename "$R5" "$R4"
      Goto plugins_preseeded

      plugins_preseed_failed:
      RmDir /r "$R5"
      plugins_preseeded:
    ${EndIf}
  ${EndIf}
  ClearErrors
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
