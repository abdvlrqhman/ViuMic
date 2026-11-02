; ViuMic installer hooks (Tauri NSIS).
;
; The ViuMic microphone is VB-CABLE by VB-Audio Software (www.vb-cable.com).
; VB-CABLE is a donationware, all participations are welcome.
; The package ships unmodified in $INSTDIR\vbcable; ViuMic only renames its endpoints at runtime,
; the same way a user can in Windows' Sound settings.

!macro NSIS_HOOK_POSTINSTALL
  ; Already installed (by an earlier ViuMic or by the user)? Nothing to do. Sysnative reaches the
  ; real System32 from this 32-bit installer.
  IfFileExists "$WINDIR\Sysnative\drivers\vbaudio_cable64_win10.sys" viumic_cable_done 0
  IfFileExists "$SYSDIR\drivers\vbaudio_cable64_win10.sys" viumic_cable_done 0
    DetailPrint "Installing the ViuMic microphone: VB-CABLE by VB-Audio (www.vb-cable.com), donationware."
    ; -i install, -h hidden. Windows still asks once to trust VB-Audio's driver.
    ExecWait '"$INSTDIR\vbcable\VBCABLE_Setup_x64.exe" -i -h' $0
    DetailPrint "VB-CABLE setup finished with code $0."
  viumic_cable_done:
!macroend
