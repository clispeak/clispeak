//! Putting the command-line tool on the PATH on Windows, and taking it off.
//!
//! Every other desktop has somewhere to put a binary that is already on the
//! PATH, or a shell profile to append a line to. Windows has neither: the
//! per-user PATH is a registry value under `HKCU\Environment`, and a process
//! that changes it has to tell the rest of the desktop, or nothing sees the
//! change until the next sign-in.
//!
//! **This is not done in NSIS**, which is where an installer would normally
//! do it, and the reason is the one this project keeps relearning. NSIS reads
//! a registry string into a fixed-size buffer — 1024 characters in the
//! standard build — and a developer's PATH is routinely longer than that. The
//! read does not fail. It truncates, and writing the truncated value back
//! *destroys the rest of the user's PATH* on a machine we do not own, silently,
//! at install time. The registry API this module uses has no such limit, and
//! it can read the value's **type**, which NSIS cannot: see [`add`].
//!
//! So the installer hook calls this binary and this binary does the work. The
//! part that decides what the new value should be is a pure function, compiled
//! on every platform and tested on Linux — because `cargo test` runs on
//! `ubuntu-latest` and nowhere else, so a test behind `cfg(windows)` is a test
//! that never runs.

/// The separator between PATH entries on Windows.
const SEP: char = ';';

/// One PATH entry, reduced to the form two entries can be compared in.
///
/// Windows paths are case-insensitive, may be quoted, and may or may not carry
/// a trailing separator — `C:\Tools\`, `c:\tools` and `"C:\Tools"` are the same
/// directory written three ways. Comparing the raw text would add a duplicate
/// entry on the second install, and would fail to remove ours on uninstall
/// because the user's copy of it is spelled differently from ours.
#[cfg_attr(not(windows), allow(dead_code))]
fn comparable(entry: &str) -> String {
    entry
        .trim()
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

/// The PATH with `dir` added, or `None` if it is already there.
///
/// Appends rather than prepends. Prepending would let this directory shadow a
/// `clispeak` the user installed some other way, which is a decision that is
/// not ours to make on their machine.
///
/// Returning `None` for "already present" is what makes the caller idempotent:
/// installing twice, or repairing an install, must not leave two copies of the
/// entry behind for the uninstaller to half-remove.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn path_with(existing: &str, dir: &str) -> Option<String> {
    let wanted = comparable(dir);
    if existing
        .split(SEP)
        .filter(|e| !e.trim().is_empty())
        .any(|e| comparable(e) == wanted)
    {
        return None;
    }
    if existing.is_empty() {
        return Some(dir.to_string());
    }
    // A PATH ending in a separator is legal and common. Appending after it
    // would produce an empty entry, which Windows reads as the current
    // directory — a real hazard, not an untidiness.
    let base = existing.trim_end_matches(SEP);
    Some(format!("{base}{SEP}{dir}"))
}

/// The PATH with `dir` removed, or `None` if it was not there.
///
/// **Everything else is preserved exactly**, including entries this function
/// would consider malformed and including empty ones. An uninstaller that
/// tidies up the rest of a PATH it did not write is a worse outcome than one
/// that leaves a stale entry: this runs on somebody's machine, once, with no
/// undo and nobody watching.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn path_without(existing: &str, dir: &str) -> Option<String> {
    let unwanted = comparable(dir);
    let mut removed = false;
    let kept: Vec<&str> = existing
        .split(SEP)
        .filter(|e| {
            if !e.trim().is_empty() && comparable(e) == unwanted {
                removed = true;
                return false;
            }
            true
        })
        .collect();
    removed.then(|| kept.join(&SEP.to_string()))
}

#[cfg(windows)]
mod imp {
    use super::{path_with, path_without};

    /// The registry key holding the per-user environment.
    ///
    /// Per-user deliberately: `HKLM` would need an administrator, and a UAC
    /// prompt is a place people stop. It is also the half we are entitled to
    /// change — this app installs for one user.
    const ENVIRONMENT: &str = "Environment";

    /// Read the PATH, apply `change`, write it back, and tell the desktop.
    ///
    /// **The value's type is read and restored**, which is the whole reason
    /// this is not three lines of NSIS. `HKCU\Environment\PATH` is normally
    /// `REG_EXPAND_SZ`, so entries like `%USERPROFILE%\bin` expand. Writing it
    /// back as a plain `REG_SZ` — which is what happens if you do not think
    /// about it — turns every such entry on the machine into a literal string
    /// that names no directory. Nothing reports an error; unrelated tools
    /// simply stop being found.
    fn edit(dir: &str, change: impl Fn(&str, &str) -> Option<String>) -> Result<bool, String> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};

        let env = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(ENVIRONMENT, KEY_READ | KEY_WRITE)
            .map_err(|e| format!("could not open HKCU\\{ENVIRONMENT}: {e}"))?;

        // A user with no PATH of their own is not an error: the machine PATH
        // from HKLM is separate and stays untouched either way.
        let (current, vtype) = match env.get_raw_value("PATH") {
            Ok(raw) => {
                let vtype = raw.vtype.clone();
                let text = <String as winreg::types::FromRegValue>::from_reg_value(&raw)
                    .map_err(|e| format!("could not read the PATH value: {e}"))?;
                (text, vtype)
            }
            Err(_) => (String::new(), REG_EXPAND_SZ),
        };

        let Some(next) = change(&current, dir) else {
            return Ok(false);
        };

        // Preserve whatever type was there, defaulting to REG_EXPAND_SZ for a
        // value we are creating — that is what Windows itself writes, and it
        // is the type that keeps `%VAR%` working for whoever edits it later.
        let vtype = match vtype {
            REG_SZ => REG_SZ,
            _ => REG_EXPAND_SZ,
        };
        let mut value = <String as winreg::types::ToRegValue>::to_reg_value(&next);
        value.vtype = vtype;
        env.set_raw_value("PATH", &value)
            .map_err(|e| format!("could not write the PATH value: {e}"))?;

        broadcast();
        Ok(true)
    }

    /// Tell every top-level window the environment changed.
    ///
    /// Without this the registry is correct and **nothing observes it**.
    /// Explorer caches the environment it hands to every process it launches,
    /// so a new terminal keeps getting the old PATH until the next sign-in —
    /// which reads exactly like the install having failed, on a machine where
    /// it succeeded.
    ///
    /// `SendMessageTimeout` rather than `SendMessage`: this is a broadcast to
    /// every top-level window on the desktop, and one hung window would
    /// otherwise hang the installer behind it. Five seconds, then give up —
    /// the value is written either way, and the worst case is the sign-in
    /// this call exists to avoid.
    #[allow(unsafe_code)]
    fn broadcast() {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        };
        use windows::core::w;

        // SAFETY: a message broadcast with no pointers of ours to outlive the
        // call. `w!` is a static UTF-16 literal, valid for the whole program,
        // and the result is deliberately ignored — a desktop that does not
        // answer is the timeout case this asks for.
        unsafe {
            let _ = SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(w!("Environment").as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                5000,
                None,
            );
        }
    }

    /// Put `dir` on the user's PATH. `Ok(false)` if it was already there.
    pub fn add(dir: &str) -> Result<bool, String> {
        edit(dir, path_with)
    }

    /// Take `dir` off the user's PATH. `Ok(false)` if it was not there.
    pub fn remove(dir: &str) -> Result<bool, String> {
        edit(dir, path_without)
    }
}

#[cfg(windows)]
pub use imp::{add, remove};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_when_absent() {
        assert_eq!(
            path_with(
                r"C:\Windows;C:\Windows\System32",
                r"C:\Users\p\AppData\Local\clispeak"
            ),
            Some(r"C:\Windows;C:\Windows\System32;C:\Users\p\AppData\Local\clispeak".into())
        );
    }

    #[test]
    fn adding_twice_changes_nothing() {
        let dir = r"C:\Tools\clispeak";
        let once = path_with(r"C:\Windows", dir).unwrap();
        assert_eq!(path_with(&once, dir), None);
    }

    /// The second install must not add a second entry because the first one
    /// was written in a different case, and the uninstaller must still find
    /// it. Windows paths are case-insensitive; string comparison is not.
    #[test]
    fn case_and_trailing_slash_are_the_same_directory() {
        assert_eq!(
            path_with(r"C:\Windows;c:\tools\clispeak\", r"C:\Tools\clispeak"),
            None
        );
        assert_eq!(
            path_without(r"C:\Windows;c:\tools\clispeak\", r"C:\Tools\clispeak"),
            Some(r"C:\Windows".to_string())
        );
    }

    #[test]
    fn quoted_entries_are_the_same_directory() {
        assert_eq!(
            path_with(r#"C:\Windows;"C:\Tools\clispeak""#, r"C:\Tools\clispeak"),
            None
        );
    }

    /// A PATH ending in `;` is legal and common. Appending blindly would leave
    /// an empty entry, which Windows resolves as the current directory — a
    /// search-path hazard, not a cosmetic one.
    #[test]
    fn a_trailing_separator_does_not_become_an_empty_entry() {
        assert_eq!(
            path_with(r"C:\Windows;", r"C:\Tools\clispeak"),
            Some(r"C:\Windows;C:\Tools\clispeak".into())
        );
    }

    #[test]
    fn an_empty_path_becomes_just_the_entry() {
        assert_eq!(
            path_with("", r"C:\Tools\clispeak"),
            Some(r"C:\Tools\clispeak".into())
        );
    }

    #[test]
    fn removing_what_is_not_there_changes_nothing() {
        assert_eq!(path_without(r"C:\Windows", r"C:\Tools\clispeak"), None);
    }

    /// The uninstaller runs once, on somebody's machine, with no undo. It may
    /// remove our entry and must not touch anything else — including the empty
    /// entry in the middle here, which we would not have written and are not
    /// entitled to clean up.
    #[test]
    fn removing_preserves_everything_else_exactly() {
        assert_eq!(
            path_without(
                r"C:\Windows;;C:\Tools\clispeak;%JAVA_HOME%\bin",
                r"C:\Tools\clispeak"
            ),
            Some(r"C:\Windows;;%JAVA_HOME%\bin".to_string())
        );
    }

    /// Unexpanded variables must survive the round trip as written. Expanding
    /// them here and writing the result back would bake one machine's values
    /// into a value that is meant to stay a template.
    #[test]
    fn unexpanded_variables_are_left_alone() {
        let path = r"%USERPROFILE%\bin;C:\Windows";
        let out = path_with(path, r"C:\Tools\clispeak").unwrap();
        assert!(out.starts_with(r"%USERPROFILE%\bin;"), "{out}");
    }
}
