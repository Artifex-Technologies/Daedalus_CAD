//! Sheet-metal flat patterns.
//!
//! A bent part is described by its cross-section: straight flanges joined by
//! bends. Flange lengths are *outside* dimensions measured to the virtual sharp
//! (the way parts are dimensioned on drawings). The flat pattern is the part
//! laid out as one strip, with a bend line at the centre of every bend.
//!
//! K-factor and bend radius depend on the material, thickness and the shop's
//! tooling, so they are inputs, never constants.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub thickness: f64,
    /// Position of the neutral axis as a fraction of thickness, 0.0 to 0.5.
    pub k_factor: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bend {
    /// Bend angle in degrees (how far the flange turns). Positive bends up,
    /// negative bends down; the sign does not change the flat length.
    pub angle_deg: f64,
    pub inside_radius: f64,
}

/// `flanges.len()` must be `bends.len() + 1`.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub flanges: Vec<f64>,
    pub bends: Vec<Bend>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BendLine {
    /// Distance from the start of the flat strip to the centre of the bend.
    pub position: f64,
    pub angle_deg: f64,
    pub inside_radius: f64,
    /// Arc length of the neutral axis through this bend.
    pub allowance: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlatPattern {
    pub length: f64,
    pub width: f64,
    pub bend_lines: Vec<BendLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SheetMetalError {
    InvalidMaterial(&'static str),
    InvalidWidth,
    FlangeCount { flanges: usize, bends: usize },
    InvalidBend { index: usize },
    /// Flange `index` is not longer than the bend setbacks that eat into it.
    FlangeTooShort { index: usize, length: f64, needed: f64 },
}

impl fmt::Display for SheetMetalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SheetMetalError::InvalidMaterial(why) => write!(f, "invalid material: {why}"),
            SheetMetalError::InvalidWidth => write!(f, "width must be positive"),
            SheetMetalError::FlangeCount { flanges, bends } => {
                write!(f, "{flanges} flanges need {} bends, got {bends}", flanges.saturating_sub(1))
            }
            SheetMetalError::InvalidBend { index } => {
                write!(f, "bend {index}: angle must be between 0 and 180 degrees (exclusive) and radius not negative")
            }
            SheetMetalError::FlangeTooShort { index, length, needed } => {
                write!(f, "flange {index} is {length} long but the bends need more than {needed}")
            }
        }
    }
}

impl std::error::Error for SheetMetalError {}

/// Arc length of the neutral axis through a bend.
pub fn bend_allowance(bend: Bend, m: Material) -> f64 {
    bend.angle_deg.abs().to_radians() * (bend.inside_radius + m.k_factor * m.thickness)
}

/// Distance from the virtual sharp back to where the bend starts, on each side.
pub fn outside_setback(bend: Bend, m: Material) -> f64 {
    (bend.angle_deg.abs().to_radians() / 2.0).tan() * (bend.inside_radius + m.thickness)
}

/// How much shorter the flat is than the sum of the two outside lengths.
pub fn bend_deduction(bend: Bend, m: Material) -> f64 {
    2.0 * outside_setback(bend, m) - bend_allowance(bend, m)
}

pub fn flat_pattern(profile: &Profile, m: Material, width: f64) -> Result<FlatPattern, SheetMetalError> {
    if !(m.thickness > 0.0) {
        return Err(SheetMetalError::InvalidMaterial("thickness must be positive"));
    }
    if !(0.0..=0.5).contains(&m.k_factor) {
        return Err(SheetMetalError::InvalidMaterial("K-factor must be between 0 and 0.5"));
    }
    if !(width > 0.0) {
        return Err(SheetMetalError::InvalidWidth);
    }
    if profile.flanges.len() != profile.bends.len() + 1 {
        return Err(SheetMetalError::FlangeCount { flanges: profile.flanges.len(), bends: profile.bends.len() });
    }
    for (index, b) in profile.bends.iter().enumerate() {
        let a = b.angle_deg.abs();
        if !(a > 0.0 && a < 180.0) || b.inside_radius < 0.0 {
            return Err(SheetMetalError::InvalidBend { index });
        }
    }

    let setbacks: Vec<f64> = profile.bends.iter().map(|&b| outside_setback(b, m)).collect();
    let mut position = 0.0;
    let mut bend_lines = Vec::with_capacity(profile.bends.len());
    for (i, &outside) in profile.flanges.iter().enumerate() {
        let before = if i > 0 { setbacks[i - 1] } else { 0.0 };
        let after = setbacks.get(i).copied().unwrap_or(0.0);
        let straight = outside - before - after;
        if straight <= 0.0 {
            return Err(SheetMetalError::FlangeTooShort { index: i, length: outside, needed: before + after });
        }
        position += straight;
        if let Some(&bend) = profile.bends.get(i) {
            let allowance = bend_allowance(bend, m);
            bend_lines.push(BendLine {
                position: position + allowance / 2.0,
                angle_deg: bend.angle_deg,
                inside_radius: bend.inside_radius,
                allowance,
            });
            position += allowance;
        }
    }
    Ok(FlatPattern { length: position, width, bend_lines })
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: Material = Material { thickness: 2.0, k_factor: 0.44 };
    const R90: Bend = Bend { angle_deg: 90.0, inside_radius: 2.0 };

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn bend_formulas_for_a_right_angle() {
        assert!(close(outside_setback(R90, M), 4.0)); // R + T at 90 degrees
        assert!(close(bend_allowance(R90, M), 4.5239));
        assert!(close(bend_deduction(R90, M), 3.4761));
    }

    #[test]
    fn l_bracket_matches_bend_deduction() {
        let p = Profile { flanges: vec![50.0, 30.0], bends: vec![R90] };
        let flat = flat_pattern(&p, M, 40.0).unwrap();
        // 80 outside total minus one bend deduction
        assert!(close(flat.length, 80.0 - 3.4761));
        assert_eq!(flat.width, 40.0);
        assert_eq!(flat.bend_lines.len(), 1);
        assert!(close(flat.bend_lines[0].position, 46.0 + 4.5239 / 2.0));
    }

    #[test]
    fn u_channel_has_two_bend_lines() {
        let p = Profile { flanges: vec![20.0, 40.0, 20.0], bends: vec![R90, R90] };
        let flat = flat_pattern(&p, M, 25.0).unwrap();
        assert!(close(flat.length, 80.0 - 2.0 * 3.4761));
        assert!(close(flat.bend_lines[0].position, 16.0 + 4.5239 / 2.0));
        assert!(close(flat.bend_lines[1].position, 16.0 + 4.5239 + 32.0 + 4.5239 / 2.0));
    }

    #[test]
    fn bend_direction_does_not_change_flat_length() {
        let down = Bend { angle_deg: -90.0, ..R90 };
        let a = flat_pattern(&Profile { flanges: vec![50.0, 30.0], bends: vec![R90] }, M, 10.0).unwrap();
        let b = flat_pattern(&Profile { flanges: vec![50.0, 30.0], bends: vec![down] }, M, 10.0).unwrap();
        assert!(close(a.length, b.length));
        assert_eq!(b.bend_lines[0].angle_deg, -90.0);
    }

    #[test]
    fn obtuse_and_acute_bends() {
        let b45 = Bend { angle_deg: 45.0, inside_radius: 1.0 };
        let setback = (22.5f64).to_radians().tan() * 3.0;
        assert!(close(outside_setback(b45, M), setback));
        let flat = flat_pattern(&Profile { flanges: vec![20.0, 20.0], bends: vec![b45] }, M, 10.0).unwrap();
        assert!(flat.length > 0.0 && flat.length < 40.0);
    }

    #[test]
    fn rejects_bad_input() {
        let p = Profile { flanges: vec![3.0, 30.0], bends: vec![R90] };
        assert!(matches!(flat_pattern(&p, M, 10.0), Err(SheetMetalError::FlangeTooShort { index: 0, .. })));
        let p = Profile { flanges: vec![50.0], bends: vec![R90] };
        assert!(matches!(flat_pattern(&p, M, 10.0), Err(SheetMetalError::FlangeCount { .. })));
        let flat_bend = Bend { angle_deg: 0.0, inside_radius: 1.0 };
        let p = Profile { flanges: vec![50.0, 50.0], bends: vec![flat_bend] };
        assert!(matches!(flat_pattern(&p, M, 10.0), Err(SheetMetalError::InvalidBend { index: 0 })));
        let ok = Profile { flanges: vec![50.0, 50.0], bends: vec![R90] };
        assert!(flat_pattern(&ok, Material { thickness: 0.0, k_factor: 0.4 }, 10.0).is_err());
        assert!(flat_pattern(&ok, Material { thickness: 2.0, k_factor: 0.9 }, 10.0).is_err());
        assert_eq!(flat_pattern(&ok, M, 0.0), Err(SheetMetalError::InvalidWidth));
    }
}
