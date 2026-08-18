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
${UnStrTrimNewLines}

Var DshWasUpgrade
Var DshProvisionTestMode
Var DshQuitAttempts

!macro NSIS_HOOK_PREINSTALL
  StrCpy $DshWasUpgrade 0
  IfFileExists "$INSTDIR\uninstall.exe" 0 +2
  StrCpy $DshWasUpgrade 1

  ; 升级必须先走旧版本的正式退出链路。超时即在复制任何新文件前终止安装。
  ${If} $DshWasUpgrade = 1
    IfFileExists "$INSTDIR\dsh-desktop.exe" 0 upgrade_quit_done
    !if "${INSTALLMODE}" == "currentUser"
      nsis_tauri_utils::FindProcessCurrentUser "${MAINBINARYNAME}.exe"
    !else
      nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
    !endif
    Pop $R0
    ${If} $R0 = 0
      ExecWait '"$INSTDIR\dsh-desktop.exe" --quit-existing' $R9
      ${If} $R9 != 0
        Abort "DeepSeek Harness Desktop could not request the running version to exit (code $R9)."
      ${EndIf}
      StrCpy $DshQuitAttempts 0
      upgrade_quit_wait:
      !if "${INSTALLMODE}" == "currentUser"
        nsis_tauri_utils::FindProcessCurrentUser "${MAINBINARYNAME}.exe"
      !else
        nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
      !endif
      Pop $R0
      ${If} $R0 != 0
        Goto upgrade_quit_done
      ${EndIf}
      IntOp $DshQuitAttempts $DshQuitAttempts + 1
      ${If} $DshQuitAttempts >= 40
        Abort "DeepSeek Harness Desktop did not exit within 10 seconds; the upgrade was cancelled."
      ${EndIf}
      Sleep 250
      Goto upgrade_quit_wait
    ${EndIf}
  ${EndIf}
  upgrade_quit_done:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $AppStartMenuFolder != ""
    CreateShortCut "$SMPROGRAMS\$AppStartMenuFolder\Uninstall DeepSeek Harness Desktop.lnk" "$INSTDIR\uninstall.exe"
  ${Else}
    CreateShortCut "$SMPROGRAMS\Uninstall DeepSeek Harness Desktop.lnk" "$INSTDIR\uninstall.exe"
  ${EndIf}

  ; Payload 安装先登记 candidate，不在安装器进程内切换 active。
  IfFileExists "$INSTDIR\payload-manifest.json" 0 legacy_preseed
  ${GetParameters} $R6
  StrCpy $R8 ""
  ClearErrors
  ${GetOptions} $R6 "/PAYLOADTESTROOT=" $R8
  ${IfNot} ${Errors}
    ${If} $R8 != ""
      ExecWait '"$INSTDIR\dsh-desktop.exe" --provision-runtime --provision-test-mode --runtime-root "$R8"' $R9
    ${Else}
      ExecWait '"$INSTDIR\dsh-desktop.exe" --provision-runtime' $R9
    ${EndIf}
  ${Else}
    ClearErrors
    ExecWait '"$INSTDIR\dsh-desktop.exe" --provision-runtime' $R9
  ${EndIf}
  ${If} $R9 != 0
    ${If} $DshWasUpgrade = 0
      Abort "DeepSeek Harness runtime provision failed with exit code $R9."
    ${Else}
      DetailPrint "Runtime provision failed with exit code $R9; keeping the previous active runtime."
    ${EndIf}
  ${ElseIf} $R8 != ""
    ; 仅安装器 smoke 会传入隔离 runtime 根；卸载前会再次规范化并限制到系统临时目录。
    FileOpen $R7 "$INSTDIR\.provision-test-runtime-root" w
    FileWrite $R7 "$R8"
    FileClose $R7
  ${EndIf}
  Goto postinstall_done

  legacy_preseed:
  ; Legacy 构建期摘要对应一个插件 store。安装时预置，避免首次启动复制完整插件树。
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
  postinstall_done:
  ClearErrors
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  StrCpy $DshProvisionTestMode 0
  IfFileExists "$INSTDIR\.provision-test-runtime-root" 0 provision_test_root_done
  StrCpy $DshProvisionTestMode 1
  FileOpen $R1 "$INSTDIR\.provision-test-runtime-root" r
  ${If} ${Errors}
    Abort "Could not read the provision test runtime marker."
  ${EndIf}
  FileRead $R1 $R2
  FileClose $R1
  ${UnStrTrimNewLines} $R2 $R2
  ${If} $R2 == ""
    Abort "Provision test runtime marker is empty."
  ${EndIf}
  ExecWait '"$INSTDIR\dsh-desktop.exe" --cleanup-provision-test-runtime --provision-test-mode --runtime-root "$R2"' $R9
  ${If} $R9 != 0
    Abort "Provision test runtime cleanup failed with exit code $R9."
  ${EndIf}
  provision_test_root_done:
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

  ; 桌面托管 runtime 总是随卸载删除；用户 profile `~/.dsh` 从不在此处理。
  ${If} $DshProvisionTestMode = 0
    SetShellVarContext current
    RmDir /r "$LOCALAPPDATA\dsh-desktop\runtime"
  ${EndIf}
  Delete "$INSTDIR\.provision-test-runtime-root"
  RMDir "$INSTDIR"

  ; 只有用户勾选删除应用数据时，才删除日志等其余 LocalAppData。
  ${If} $DeleteAppDataCheckboxState = 1
    RmDir /r "$LOCALAPPDATA\dsh-desktop"
  ${EndIf}

  ; Cleanup failures here (e.g. RMDir on a non-empty shared folder) should
  ; not change the uninstaller's success status.
  ClearErrors
!macroend
