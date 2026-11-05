//! Lengths as people write them. The model is in metres; what is shown and typed is in the chosen units: millimetres,
//! centimetres or metres for parts and metric buildings, inches or feet and inches for imperial ones.
//!
//! Typing follows SketchUp: a bare number is in the chosen units, and a unit written after it wins (`300mm`, `1.2m`, `30cm`,
//! `6"`, `6in`, `4'`, `4' 6"`, `4'6 1/2"`), so a value can be typed in any unit whatever is shown.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Millimetres,
    Centimetres,
    Metres,
    Inches,
    FeetInches,
}

pub const ALL: [Units; 5] = [Units::Millimetres, Units::Centimetres, Units::Metres, Units::Inches, Units::FeetInches];

const INCH: f64 = 0.0254;

impl Units {
    pub fn name(self) -> &'static str {
        match self {
            Units::Millimetres => "Millimetres",
            Units::Centimetres => "Centimetres",
            Units::Metres => "Metres",
            Units::Inches => "Inches",
            Units::FeetInches => "Feet and inches",
        }
    }

    /// The word kept in settings files.
    pub fn key(self) -> &'static str {
        match self {
            Units::Millimetres => "mm",
            Units::Centimetres => "cm",
            Units::Metres => "m",
            Units::Inches => "in",
            Units::FeetInches => "ft-in",
        }
    }

    pub fn from_key(k: &str) -> Option<Units> {
        ALL.into_iter().find(|u| u.key() == k.trim())
    }

    /// Metres in one of this unit (feet and inches: one inch, the unit a bare number means).
    fn metres(self) -> f64 {
        match self {
            Units::Millimetres => 0.001,
            Units::Centimetres => 0.01,
            Units::Metres => 1.0,
            Units::Inches | Units::FeetInches => INCH,
        }
    }
}

fn trim_zeros(s: String) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// A length in metres, written in `units`: `300 mm`, `1.25 m`, `6.5"`, `4' 6 1/2"` (feet and inches to the nearest 1/16").
pub fn format(metres: f32, units: Units) -> String {
    let m = metres as f64;
    let sign = if m < 0.0 { "-" } else { "" };
    let a = m.abs();
    match units {
        Units::Millimetres => format!("{} mm", trim_zeros(format!("{:.2}", m / 0.001))),
        Units::Centimetres => format!("{} cm", trim_zeros(format!("{:.3}", m / 0.01))),
        Units::Metres => format!("{} m", trim_zeros(format!("{:.4}", m))),
        Units::Inches => format!("{}\"", trim_zeros(format!("{:.3}", m / INCH))),
        Units::FeetInches => {
            let sixteenths = (a / INCH * 16.0).round() as i64;
            let (feet, rest) = (sixteenths / (12 * 16), sixteenths % (12 * 16));
            let (inches, frac) = (rest / 16, rest % 16);
            let frac = if frac == 0 {
                String::new()
            } else {
                let g = gcd(frac, 16);
                format!(" {}/{}", frac / g, 16 / g)
            };
            // a fraction of an inch alone reads 3/4", not 0 3/4"
            let inch_part = if inches == 0 && !frac.is_empty() { frac.trim_start().to_string() } else { format!("{inches}{frac}") };
            if feet == 0 {
                format!("{sign}{inch_part}\"")
            } else {
                format!("{sign}{feet}' {inch_part}\"")
            }
        }
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// A number, possibly with a fraction after a space (`6 1/2`) or alone (`1/2`).
fn number(t: &str) -> Option<f64> {
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    let mut total = 0.0;
    for part in t.split_whitespace() {
        total += match part.split_once('/') {
            Some((a, b)) => {
                let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
                if b == 0.0 {
                    return None;
                }
                a / b
            }
            None => part.parse::<f64>().ok()?,
        };
    }
    total.is_finite().then_some(total)
}

/// A typed length in metres. A bare number is in `units`; a written unit wins.
pub fn parse(text: &str, units: Units) -> Option<f32> {
    let t = text.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r.trim_start()),
        None => (false, t),
    };
    let v = if let Some((ft, rest)) = t.split_once('\'') {
        // feet, maybe followed by inches: 4', 4'6", 4' 6 1/2"
        let feet = number(ft)?;
        let rest = rest.trim().trim_end_matches('"').trim_end_matches("in").trim();
        let inches = if rest.is_empty() { 0.0 } else { number(rest)? };
        (feet * 12.0 + inches) * INCH
    } else {
        let lower = t.to_ascii_lowercase();
        let units_written: [(&str, f64); 7] = [("mm", 0.001), ("cm", 0.01), ("km", 1000.0), ("m", 1.0), ("in", INCH), ("\"", INCH), ("ft", 12.0 * INCH)];
        match units_written.iter().find(|(s, _)| lower.ends_with(s)) {
            Some((s, factor)) => number(&t[..t.len() - s.len()])? * factor,
            None => number(t)? * units.metres(),
        }
    };
    let v = if neg { -v } else { v };
    (v.is_finite()).then_some(v as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Option<f32>, b: f64) -> bool {
        a.is_some_and(|a| (a as f64 - b).abs() < 1e-6)
    }

    #[test]
    fn typing_in_any_unit() {
        assert!(close(parse("300", Units::Millimetres), 0.3));
        assert!(close(parse("300mm", Units::Metres), 0.3), "a written unit wins");
        assert!(close(parse("1.2 m", Units::Millimetres), 1.2));
        assert!(close(parse("30cm", Units::Inches), 0.3));
        assert!(close(parse("6\"", Units::Metres), 6.0 * 0.0254));
        assert!(close(parse("6in", Units::Metres), 6.0 * 0.0254));
        assert!(close(parse("2ft", Units::Metres), 24.0 * 0.0254));
        assert!(close(parse("4'", Units::Metres), 48.0 * 0.0254));
        assert!(close(parse("4' 6\"", Units::Metres), 54.0 * 0.0254));
        assert!(close(parse("4'6 1/2\"", Units::Metres), 54.5 * 0.0254));
        assert!(close(parse("6 1/2", Units::FeetInches), 6.5 * 0.0254), "a bare number in feet and inches is inches");
        assert!(close(parse("-1.5", Units::Metres), -1.5));
        assert!(parse("abc", Units::Metres).is_none());
        assert!(parse("1/0", Units::Metres).is_none());
        assert!(parse("", Units::Metres).is_none());
    }

    #[test]
    fn showing_in_the_chosen_unit() {
        assert_eq!(format(0.3, Units::Millimetres), "300 mm");
        assert_eq!(format(0.0125, Units::Millimetres), "12.5 mm");
        assert_eq!(format(1.5, Units::Metres), "1.5 m");
        assert_eq!(format(0.3, Units::Centimetres), "30 cm");
        assert_eq!(format(6.5 * 0.0254, Units::Inches), "6.5\"");
        assert_eq!(format(54.5 * 0.0254, Units::FeetInches), "4' 6 1/2\"");
        assert_eq!(format(48.0 * 0.0254, Units::FeetInches), "4' 0\"");
        assert_eq!(format(0.75 * 0.0254, Units::FeetInches), "3/4\"");
        assert_eq!(format(-12.0 * 0.0254, Units::FeetInches), "-1' 0\"");
    }

    #[test]
    fn a_shown_value_types_back_the_same() {
        for u in ALL {
            for v in [0.3_f32, 1.5, 0.0127, 3.0] {
                let back = parse(&format(v, u), Units::Metres).unwrap();
                let step = if u == Units::FeetInches { 0.0254 / 16.0 } else { 1e-4 };
                assert!((back - v).abs() <= step as f32, "{} in {:?} came back as {back}", v, u);
            }
        }
    }
}
