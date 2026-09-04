; Hooks into Tauri's NSIS installer.
;
; Deliberately almost empty. Every decision about the user's PATH is made in
; Rust, in `src/winpath.rs`, and this file does nothing but call the binary
; that was just installed. That split is the point:
;
;   * NSIS reads a registry string into a fixed 1024-character buffer. A
;     developer's PATH is routinely longer. The read does not fail, it
;     truncates -- and writing the truncated value back destroys the rest of
;     someone's PATH, silently, on a machine we do not own.
;   * NSIS cannot see a value's *type*, so it cannot tell REG_EXPAND_SZ from
;     REG_SZ, and writing the wrong one turns every %VAR% entry on the machine
;     into a literal string naming no directory.
;   * Nothing here can be tested. The Rust side is a pure function with tests
;     that run on Linux, which is the only place `cargo test` runs at all.
;
; So the rule for this file is: if a change to it needs more than one line of
; logic, it belongs on the other side of the call.
;
; ${MAINBINARYNAME} is Tauri's own define for the app executable, used rather
; than the name written out. Tauri names that binary after the *cargo* binary
; unless `mainBinaryName` overrides it, so it is `clispeak-app.exe` and not
; `clispeak.exe` -- but guessing it here would fail the way this repository
; keeps being failed: `nsExec` on a path that does not exist returns an error
; nobody reads, and the install would finish looking exactly like a success
; with no command on the PATH.
;
; The app also adds itself to the PATH on every launch, so if these hooks do
; not run at all -- an antivirus blocking the child process, a zip somebody
; extracted by hand -- the first launch repairs it. What is lost in that case
; is only the removal on uninstall.

!macro NSIS_HOOK_POSTINSTALL
  ; Put the install directory on the user's PATH, so `clispeak` is a command.
  ; `ExecToLog` rather than `Exec` so whatever it says lands in the installer's
  ; details pane instead of a window nobody sees.
  ;
  ; The status is deliberately ignored. A PATH that could not be written leaves
  ; a working app and a missing command -- worth saying, not worth failing an
  ; otherwise good install for, and certainly not worth rolling back over.
  DetailPrint "Adding clispeak to your PATH"
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --add-to-path'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Before the files go, while the binary that knows how still exists.
  ; It removes only our own entry and preserves the rest of the value exactly.
  DetailPrint "Removing clispeak from your PATH"
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --remove-from-path'
  Pop $0
!macroend
