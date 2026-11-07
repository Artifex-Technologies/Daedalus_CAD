//! Bend tables: a fabricator's own values per material and thickness, kept as a CSV file (the viewer keeps one in
//! `~/.config/daedalus/bend-table.csv`). One row per line:
//!
//! ```text
//! material, thickness, inside radius, K-factor
//! Mild steel, 0.060", 0.030", 0.42
//! Aluminium 5052, 2 mm, 2.5 mm, BD 3.9
//! ```
//!
//! Lengths are mm unless they carry a unit (`0.060"`, `1.5 mm`, `0.06in`). Shops publish the bend either as a K-factor, as
//! the bend deduction for a 90 degree bend (`BD 3.9`) or as its bend allowance (`BA 3.4`); the last two are turned into the
//! K-factor here, so every part is made the same way. Fields are separated by commas (or semicolons, or tabs) and may be
//! quoted. Blank lines, `#` comments and a heading line are passed over; a row that does not read is reported, not guessed.

use crate::sheet::RuleValues;
use daedalus_core::units::{self, Units};
use std::f64::consts::FRAC_PI_2;

/// One row of a bend table (mm).
#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    pub material: String,
    pub thickness: f64,
    /// Inside bend radius.
    pub radius: f64,
    pub k_factor: f64,
}

impl TableRow {
    /// How a part made of it is named: the material, its thickness and radius.
    pub fn name(&self) -> String {
        format!("{} {} mm, R {}", self.material, fmt(self.thickness), fmt(self.radius))
    }

    /// The rule values for it (reliefs and gaps from the thickness, as for the built-in materials).
    pub fn rule(&self) -> RuleValues {
        RuleValues { k_factor: self.k_factor, bend_radius: self.radius, ..RuleValues::steel(self.thickness) }
    }
}

fn fmt(x: f64) -> String {
    let s = format!("{x:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The K-factor of a 90 degree bend whose allowance (the length of its neutral line) is `ba`: BA = pi/2 (R + K T).
pub fn k_from_allowance(thickness: f64, radius: f64, ba: f64) -> f64 {
    (ba / FRAC_PI_2 - radius) / thickness
}

/// The K-factor of a 90 degree bend whose deduction is `bd`: BD = 2 (R + T) - BA, the outside setback (R + T) twice less the
/// allowance.
pub fn k_from_deduction(thickness: f64, radius: f64, bd: f64) -> f64 {
    k_from_allowance(thickness, radius, 2.0 * (radius + thickness) - bd)
}

/// A line's fields, split on `sep` and unquoted (`""` inside quotes is one `"`).
fn fields(line: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut quoted, mut chars) = (String::new(), false, line.chars().peekable());
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            // a quote opens a field only at its start: `0.060"` is inches
            '"' if quoted => quoted = false,
            '"' if cur.trim().is_empty() => {
                cur.clear();
                quoted = true;
            }
            c if c == sep && !quoted => out.push(std::mem::take(&mut cur).trim().to_string()),
            c => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn length(text: &str) -> Option<f64> {
    units::parse(text, Units::Millimetres).map(|m| m * 1000.0).filter(|x| x.is_finite() && *x >= 0.0)
}

/// The bend value: a K-factor, or `BD`/`BA` and a length.
fn k_value(text: &str, thickness: f64, radius: f64) -> Result<f64, String> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    for (tag, from) in [("bd", k_from_deduction as fn(f64, f64, f64) -> f64), ("ba", k_from_allowance)] {
        if let Some(rest) = lower.strip_prefix(tag) {
            let rest = rest.trim_start_matches([' ', '=', ':']);
            let v = length(&t[t.len() - rest.len()..]).ok_or_else(|| format!("{} {:?} is not a length", tag.to_uppercase(), rest))?;
            return Ok(from(thickness, radius, v));
        }
    }
    t.parse::<f64>().map_err(|_| format!("{t:?} is not a K-factor (a number, or BD or BA and a length)"))
}

/// The rows of a bend table, and what was passed over that looked like a row (line number and why).
pub fn read(text: &str) -> (Vec<TableRow>, Vec<String>) {
    let (mut rows, mut skipped) = (Vec::new(), Vec::new());
    for (n, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let sep = if line.contains('\t') {
            '\t'
        } else if line.contains(';') && !line.contains(',') {
            ';'
        } else {
            ','
        };
        let f = fields(line, sep);
        let at = |m: String| format!("line {}: {m}", n + 1);
        if f.len() < 4 {
            skipped.push(at("needs four fields: material, thickness, inside radius, K-factor".into()));
            continue;
        }
        let (Some(thickness), Some(radius)) = (length(&f[1]), length(&f[2])) else {
            // a heading names its columns
            if !f[0].to_ascii_lowercase().contains("material") {
                skipped.push(at(format!("the thickness {:?} or radius {:?} is not a length", f[1], f[2])));
            }
            continue;
        };
        if f[0].is_empty() || thickness <= 0.0 {
            skipped.push(at("a material and a thickness over 0 are needed".into()));
            continue;
        }
        match k_value(&f[3], thickness, radius) {
            Ok(k) if k > 0.0 && k <= 1.0 => rows.push(TableRow { material: f[0].clone(), thickness, radius, k_factor: k }),
            Ok(k) => skipped.push(at(format!("that makes a K-factor of {k:.3}, outside 0 to 1"))),
            Err(e) => skipped.push(at(e)),
        }
    }
    (rows, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_read_with_units_quotes_and_each_way_of_giving_the_bend() {
        let text = "\u{feff}Material,Thickness,Radius,K\n\
            # my shop's numbers\n\
            Mild steel, 0.060\", 0.030\", 0.42\n\
            \"Steel, galvanised\",1.5 mm,1.5mm,0.44\n\
            Aluminium 5052; 2; 2.5; BD 3.9\n\
            Stainless\t3\t3\tba=6.1\n\
            \n\
            Brass, 1, 1\n\
            Copper, thick, 1, 0.4\n\
            Lead, 1, 1, 7\n";
        let (rows, skipped) = read(text);
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert_eq!(rows[0].material, "Mild steel");
        assert!((rows[0].thickness - 1.524).abs() < 1e-9 && (rows[0].radius - 0.762).abs() < 1e-9 && rows[0].k_factor == 0.42);
        assert_eq!(rows[1].material, "Steel, galvanised");
        // BD 3.9 at T 2, R 2.5: BA = 2 (4.5) - 3.9 = 5.1, K = (5.1 / (pi/2) - 2.5) / 2
        assert!((rows[2].k_factor - (5.1 / FRAC_PI_2 - 2.5) / 2.0).abs() < 1e-12, "{}", rows[2].k_factor);
        assert!((rows[3].k_factor - (6.1 / FRAC_PI_2 - 3.0) / 3.0).abs() < 1e-12);
        // the heading and the comment pass quietly; the three bad rows are named by line
        assert_eq!(skipped.len(), 3, "{skipped:?}");
        assert!(skipped[0].starts_with("line 8:") && skipped[1].starts_with("line 9:") && skipped[2].starts_with("line 10:"), "{skipped:?}");
        assert_eq!(rows[2].name(), "Aluminium 5052 2 mm, R 2.5");
        let rule = rows[2].rule().checked().unwrap();
        assert_eq!((rule.thickness, rule.bend_radius, rule.k_factor), (2.0, 2.5, rows[2].k_factor));
    }

    #[test]
    fn deduction_and_allowance_agree_with_the_k_factor_they_came_from() {
        let (t, r, k) = (1.9, 2.0, 0.44);
        let ba = FRAC_PI_2 * (r + k * t);
        assert!((k_from_allowance(t, r, ba) - k).abs() < 1e-12);
        assert!((k_from_deduction(t, r, 2.0 * (r + t) - ba) - k).abs() < 1e-12);
    }
}
