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

  ; "Delete the application data" (0.13). Tauri's own box removes the folder named after the bundle id;
  ; SDC's chats, hosts and settings are in %APPDATA%\sdc and its keys in the Windows credential store, so an
  ; uninstall-and-reinstall used to bring every one of them back. The daemon forgets its own keys, then the
  ; folder goes. An update (/UPDATE) never shows the box, so an update never erases anything.
  ${If} $DeleteAppDataCheckboxState = 1
    nsExec::Exec '"$INSTDIR\sdcd.exe" --forget-everything'
    Pop $0
    RMDir /r "$APPDATA\sdc"
  ${EndIf}
!macroend
