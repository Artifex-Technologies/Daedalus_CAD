//! The viewer's own settings, kept between sessions in `~/.config/daedalus/settings.conf` (`key = value` lines; unknown lines are
//! kept). Today: the units lengths are shown and typed in.

use daedalus_core::units::Units;
use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("daedalus").join("settings.conf"))
}

/// The value of `key` in settings text.
pub fn get(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split('#').next()?.split_once('=')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })
}

/// The text with `key` set to `value` (replaced where it was, else added at the end).
pub fn set(text: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| match l.split_once('=') {
            Some((k, _)) if k.trim() == key && !l.trim_start().starts_with('#') => {
                found = true;
                format!("{key} = {value}")
            }
            _ => l.to_string(),
        })
        .collect();
    if !found {
        out.push(format!("{key} = {value}"));
    }
    out.join("\n") + "\n"
}

pub fn load_units() -> Units {
    path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| get(&t, "units")).and_then(|v| Units::from_key(&v)).unwrap_or(Units::Metres)
}

pub fn save_units(u: Units) {
    let Some(p) = path() else { return };
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&p, set(&text, "units", u.key()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_text() {
        let t = set("# Daedalus\nother = 1\n", "units", "mm");
        assert_eq!(get(&t, "units").as_deref(), Some("mm"));
        assert_eq!(get(&t, "other").as_deref(), Some("1"), "other lines are kept");
        let t = set(&t, "units", "ft-in");
        assert_eq!(get(&t, "units").as_deref(), Some("ft-in"));
        assert_eq!(t.matches("units").count(), 1, "replaced, not added twice");
        assert_eq!(Units::from_key("ft-in"), Some(Units::FeetInches));
    }
}
