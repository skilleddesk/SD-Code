; Installer hooks (0.11.6).
;
; An update installed while SDC was open - or in the eight seconds its daemon outlives the window - found
; `sdcd.exe` running. Windows keeps a running program's file locked, so the installer left the *previous*
; daemon in place next to the new app: the status bar read `v0.11.5 · sdcd 0.11.4`, the bridge replaced the
; daemon with the same old binary, and the window came up with no providers, no chats and `offline`.
; That is the "all my providers were removed" report, measured on the machine it came from.
;
; So the installer stops both programs first. `/T` takes the daemon's children with it (a CLI turn, the
; askpass helper); the SSH sign-in is Git's `ssh.exe` and is not touched.
!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec 'taskkill /F /T /IM sdc.exe'
  Pop $0
  nsExec::Exec 'taskkill /F /T /IM sdcd.exe'
  Pop $0
  Sleep 1000
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /F /T /IM sdc.exe'
  Pop $0
  nsExec::Exec 'taskkill /F /T /IM sdcd.exe'
  Pop $0
  Sleep 1000

  ; "Delete the application data" (0.13, made safe in 0.15.5). 0.13 erased %APPDATA%\sdc and the keys in
  ; the credential store whenever this box's state read 1. On 2026-09-29 a silent install of 0.15.4 over
  ; 0.15.3 (`setup.exe /S`, no `/UPDATE`) ran the old uninstaller as a real uninstall, the state read 1
  ; with nobody at the screen, and every chat, host, setting and API key was gone. How the state came to be
  ; 1 is not proven, so nothing here depends on it being right:
  ;   * an update (/UPDATE) or a silent / passive run touches nothing at all;
  ;   * otherwise the folder is only MOVED aside, to %APPDATA%\sdc-removed-<ticks>, never erased - a
  ;     reinstall does not see it, and a person who did not mean it gets everything back by renaming it;
  ;   * the keys are never removed here. Settings → Erase all data is the way to remove them on purpose.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
  ${AndIfNot} ${Silent}
    System::Call 'kernel32::GetTickCount() i .r1'
    Rename "$APPDATA\sdc" "$APPDATA\sdc-removed-$1"
  ${EndIf}
!macroend
