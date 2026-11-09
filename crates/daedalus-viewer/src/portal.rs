//! The desktop's own open and save dialogs, through the XDG desktop portal (`org.freedesktop.portal.FileChooser`): Minos on
//! Yggdrasil, GNOME's or KDE's dialog elsewhere, so files are picked the way they are in every other program. The request
//! runs on its own thread (the dialog can stay open for minutes) and the answer comes back to the window as an event. Where
//! there is no portal, the built-in browser (filedlg.rs) is used instead.

use crate::filedlg::Purpose;
use daedalus_io::formats::Format;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use zbus::zvariant::{OwnedValue, Value};

/// How a request ended.
#[derive(Debug, Clone)]
pub enum Answer {
    Chosen(PathBuf),
    Cancelled,
    /// No portal answered (not running, or no file chooser behind it): use the built-in browser.
    NoPortal,
}

/// A save format's patterns: its endings.
fn globs(f: Format) -> Vec<&'static str> {
    match f {
        Format::Step => vec!["*.step", "*.stp"],
        Format::Native => vec!["*.ddls"],
        Format::Obj => vec!["*.obj"],
        Format::Stl => vec!["*.stl"],
        Format::Dae => vec!["*.dae"],
        Format::Dxf => vec!["*.dxf"],
    }
}

/// The filters offered, the first one selected: (label, glob patterns).
fn filters(purpose: Purpose) -> Vec<(&'static str, Vec<&'static str>)> {
    let models = vec!["*.ddls", "*.daedalus", "*.step", "*.stp", "*.skp", "*.dskp", "*.stl", "*.obj", "*.dxf", "*.dae"];
    match purpose {
        Purpose::Open => vec![
            ("Models Daedalus opens", models),
            ("Daedalus models (.ddls, .daedalus)", vec!["*.ddls", "*.daedalus"]),
            ("STEP (.step, .stp)", vec!["*.step", "*.stp"]),
            ("SketchUp models (.skp)", vec!["*.skp", "*.dskp"]),
            ("All files", vec!["*"]),
        ],
        Purpose::Import => vec![
            ("STEP, STL, OBJ, DXF, COLLADA", vec!["*.step", "*.stp", "*.stl", "*.obj", "*.dxf", "*.dae"]),
            ("SketchUp models (.skp)", vec!["*.skp", "*.dskp"]),
            ("All files", vec!["*"]),
        ],
        // the format it starts in first (selected), then the others Save writes
        Purpose::SaveAs(first) => std::iter::once(first).chain(Format::SAVE.into_iter().filter(|&f| f != first)).map(|f| (f.label(), globs(f))).collect(),
        Purpose::Copy => vec![(Format::Native.label(), globs(Format::Native))],
        Purpose::Texture => vec![("Images", vec!["*.jpg", "*.jpeg", "*.png", "*.bmp"]), ("All files", vec!["*"])],
        Purpose::Step | Purpose::PartStep => vec![("STEP (.step)", vec!["*.step", "*.stp"])],
        Purpose::FlatDxf => vec![("DXF flat pattern (.dxf)", vec!["*.dxf"])],
        Purpose::BendTable => vec![("Bend tables (.csv)", vec!["*.csv", "*.txt"]), ("All files", vec!["*"])],
        Purpose::Export => vec![
            ("STL", vec!["*.stl"]),
            ("OBJ", vec!["*.obj"]),
            ("DXF", vec!["*.dxf"]),
            ("COLLADA (.dae)", vec!["*.dae"]),
        ],
    }
}

fn title(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Open => "Open a model",
        Purpose::SaveAs(_) => "Save the model",
        Purpose::Copy => "Save a full copy of the model (.ddls)",
        Purpose::Import => "Import a model",
        Purpose::Export => "Export the model as",
        Purpose::Texture => "Choose an image for the texture",
        Purpose::Step => "Export exact solids for fabrication (STEP)",
        Purpose::PartStep => "Export the part as STEP",
        Purpose::FlatDxf => "Export the flat pattern as DXF",
        Purpose::BendTable => "Choose your fabricator's bend table (CSV)",
    }
}

/// A `file://` URI's path, its %-escapes decoded.
pub fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Asks the portal for a file; `done` gets the answer, on the request's own thread.
pub fn ask(purpose: Purpose, start: Option<&Path>, name: &str, done: impl FnOnce(Answer) + Send + 'static) {
    let folder = start.and_then(|p| p.parent()).filter(|d| d.is_dir()).map(Path::to_path_buf);
    let name = name.to_string();
    std::thread::Builder::new()
        .name("file-chooser".into())
        .spawn(move || done(request(purpose, folder, &name).unwrap_or(Answer::NoPortal)))
        .map(|_| ())
        .unwrap_or(());
}

fn request(purpose: Purpose, folder: Option<PathBuf>, name: &str) -> Option<Answer> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let token = format!("daedalus_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let sender = conn.unique_name()?.trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    // listen for the answer before asking, so a quick one is not missed
    let req = zbus::blocking::Proxy::new(&conn, "org.freedesktop.portal.Desktop", path, "org.freedesktop.portal.Request").ok()?;
    let mut answers = req.receive_signal("Response").ok()?;
    let filters: Vec<(&str, Vec<(u32, &str)>)> = filters(purpose).into_iter().map(|(l, pats)| (l, pats.into_iter().map(|p| (0u32, p)).collect())).collect();
    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("handle_token", Value::from(token.as_str()));
    opts.insert("modal", Value::from(true));
    opts.insert("current_filter", Value::from(filters[0].clone()));
    opts.insert("filters", Value::from(filters));
    let saving = matches!(purpose, Purpose::SaveAs(_) | Purpose::Copy | Purpose::Export | Purpose::Step | Purpose::PartStep | Purpose::FlatDxf);
    if saving && !name.is_empty() {
        opts.insert("current_name", Value::from(name));
    }
    if let Some(dir) = folder {
        use std::os::unix::ffi::OsStrExt;
        let mut bytes = dir.as_os_str().as_bytes().to_vec();
        bytes.push(0);
        opts.insert("current_folder", Value::from(bytes));
    }
    let method = if saving { "SaveFile" } else { "OpenFile" };
    conn.call_method(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.FileChooser"), method, &("", title(purpose), opts))
        .ok()?;
    let msg = answers.next()?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().ok()?;
    if code != 0 {
        return Some(Answer::Cancelled);
    }
    let uris: Vec<String> = results.get("uris").and_then(|v| Vec::<String>::try_from(v.clone()).ok()).unwrap_or_default();
    // a name typed without an ending takes the one of the filter chosen (the format picked in the dialog)
    let picked = results.get("current_filter").and_then(|v| <(String, Vec<(u32, String)>)>::try_from(v.clone()).ok()).and_then(|(_, pats)| filter_ending(&pats));
    Some(match uris.first().and_then(|u| path_from_uri(u)) {
        Some(mut p) => {
            if p.extension().is_none() {
                if let Some(e) = picked {
                    p.set_extension(e);
                }
            }
            Answer::Chosen(p)
        }
        None => Answer::Cancelled,
    })
}

/// The ending a filter's first pattern names (`*.step` -> `step`; none for `*`).
fn filter_ending(patterns: &[(u32, String)]) -> Option<String> {
    patterns.iter().find_map(|(kind, p)| (*kind == 0).then(|| p.strip_prefix("*.")).flatten()).filter(|e| !e.is_empty() && !e.contains('*')).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_offers_every_format_the_one_it_starts_in_first() {
        let f = filters(Purpose::SaveAs(Format::Obj));
        assert_eq!(f.len(), 6);
        assert_eq!(f[0], (Format::Obj.label(), vec!["*.obj"]));
        assert_eq!(f[1].1, vec!["*.step", "*.stp"]);
        assert_eq!(filters(Purpose::SaveAs(Format::Step))[0].0, "STEP (.step): opens in every CAD program");
        assert!(filters(Purpose::Open)[0].1.contains(&"*.ddls") && filters(Purpose::Open)[0].1.contains(&"*.stp"));
        assert_eq!(filter_ending(&[(0, "*.step".into()), (0, "*.stp".into())]).as_deref(), Some("step"));
        assert_eq!(filter_ending(&[(0, "*".into())]), None);
    }

    #[test]
    fn uris_become_paths() {
        assert_eq!(path_from_uri("file:///home/m/3D%20Objects/Condo%203.skp"), Some(PathBuf::from("/home/m/3D Objects/Condo 3.skp")));
        assert_eq!(path_from_uri("file:///tmp/a.daedalus"), Some(PathBuf::from("/tmp/a.daedalus")));
        assert_eq!(path_from_uri("https://x"), None);
    }
}
