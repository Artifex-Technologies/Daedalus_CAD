//! Light or dark chrome, following the desktop. The menus, toolbar and status bar take the system's style;
//! the modelling area stays white for now (its background and lighting are still to be decided).
//!
//! On Linux the style comes from the XDG desktop portal (`org.freedesktop.appearance color-scheme`), which
//! GNOME, KDE and Yggdrasil all answer; a background thread listens for its `SettingChanged` signal so a
//! switch takes effect at once. winit reports nothing there. On Windows winit reports it, with
//! `WindowEvent::ThemeChanged` on a switch.

/// The portal's color-scheme value: 1 is "prefer dark"; 0 (no preference) and 2 (prefer light) are light.
pub fn is_dark_scheme(value: u32) -> bool {
    value == 1
}

#[cfg(target_os = "linux")]
mod portal {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    const NAMESPACE: &str = "org.freedesktop.appearance";
    const KEY: &str = "color-scheme";

    fn proxy(conn: &Connection) -> zbus::Result<Proxy<'_>> {
        Proxy::new(conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings")
    }

    /// A u32 inside however many variant layers the portal wraps it in (the older `Read` adds one).
    fn as_u32(v: &Value) -> Option<u32> {
        match v {
            Value::U32(n) => Some(*n),
            Value::Value(inner) => as_u32(inner),
            _ => None,
        }
    }

    /// Whether the desktop prefers dark, read once. None when there is no portal.
    pub fn read(conn: &Connection) -> Option<bool> {
        let p = proxy(conn).ok()?;
        let v: OwnedValue = p
            .call("ReadOne", &(NAMESPACE, KEY))
            .or_else(|_| p.call("Read", &(NAMESPACE, KEY)))
            .ok()?;
        as_u32(&v).map(super::is_dark_scheme)
    }

    /// Reads the style now and calls `changed` from a background thread whenever it changes.
    pub fn watch(changed: impl Fn(bool) + Send + 'static) -> Option<bool> {
        let conn = Connection::session().ok()?;
        let now = read(&conn);
        std::thread::Builder::new()
            .name("theme".into())
            .spawn(move || {
                let Ok(p) = proxy(&conn) else { return };
                let Ok(signals) = p.receive_signal("SettingChanged") else { return };
                for msg in signals {
                    let Ok((ns, key, value)) = msg.body().deserialize::<(String, String, OwnedValue)>() else { continue };
                    if ns == NAMESPACE && key == KEY {
                        if let Some(n) = as_u32(&value) {
                            changed(super::is_dark_scheme(n));
                        }
                    }
                }
            })
            .ok()?;
        now
    }
}

/// The desktop's style now (true: dark), and `changed` called on every switch after. None when it cannot be
/// told (then the window's own report, or light).
#[cfg(target_os = "linux")]
pub fn watch(changed: impl Fn(bool) + Send + 'static) -> Option<bool> {
    portal::watch(changed)
}

#[cfg(not(target_os = "linux"))]
pub fn watch(_changed: impl Fn(bool) + Send + 'static) -> Option<bool> {
    None
}

/// The chrome's look for a style.
pub fn visuals(dark: bool) -> egui::Visuals {
    if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_prefer_dark_is_dark() {
        assert!(is_dark_scheme(1));
        assert!(!is_dark_scheme(0));
        assert!(!is_dark_scheme(2));
    }
}
