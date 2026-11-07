//! Materials: stiffness (Young's modulus, Poisson's ratio), strength (yield) and density, in SI units.
//!
//! The presets are typical handbook values for the common grades, not a supplier's certified numbers: a part that matters is
//! checked against the material's own data sheet.

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// Young's modulus, Pa.
    pub youngs_modulus: f64,
    pub poissons_ratio: f64,
    /// Yield strength, Pa: the stress at which the part bends for good.
    pub yield_strength: f64,
    /// kg/m³ (for the part's own weight).
    pub density: f64,
}

impl Material {
    pub fn new(name: &str, e_gpa: f64, nu: f64, yield_mpa: f64, density: f64) -> Material {
        Material { name: name.to_string(), youngs_modulus: e_gpa * 1e9, poissons_ratio: nu, yield_strength: yield_mpa * 1e6, density }
    }

    /// Typical values for the grades most often cut and bent.
    pub fn presets() -> Vec<Material> {
        vec![
            Material::new("Mild steel (A36)", 200.0, 0.26, 250.0, 7850.0),
            Material::new("Cold-rolled steel (1008)", 200.0, 0.29, 285.0, 7870.0),
            Material::new("Stainless steel (304)", 193.0, 0.29, 215.0, 8000.0),
            Material::new("Aluminium (6061-T6)", 68.9, 0.33, 276.0, 2700.0),
            Material::new("Aluminium (5052-H32)", 70.3, 0.33, 193.0, 2680.0),
            Material::new("Titanium (Grade 5)", 113.8, 0.34, 880.0, 4430.0),
        ]
    }

    /// The 6x6 elasticity matrix (Voigt order xx, yy, zz, xy, yz, zx; engineering shear strains).
    pub fn elasticity(&self) -> [[f64; 6]; 6] {
        let (e, nu) = (self.youngs_modulus, self.poissons_ratio);
        let lambda = e * nu / ((1.0 + nu) * (1.0 - 2.0 * nu));
        let mu = e / (2.0 * (1.0 + nu));
        let mut d = [[0.0; 6]; 6];
        for i in 0..3 {
            for j in 0..3 {
                d[i][j] = lambda;
            }
            d[i][i] = lambda + 2.0 * mu;
            d[i + 3][i + 3] = mu;
        }
        d
    }

    /// Whether the numbers can be solved with (positive stiffness, Poisson's ratio below 0.5).
    pub fn check(&self) -> Result<(), String> {
        if !(self.youngs_modulus > 0.0) {
            return Err("the material's Young's modulus must be above zero".into());
        }
        if !(self.poissons_ratio > -0.99 && self.poissons_ratio < 0.49) {
            return Err("the material's Poisson's ratio must be between -0.99 and 0.49".into());
        }
        if !(self.yield_strength > 0.0) {
            return Err("the material's yield strength must be above zero".into());
        }
        Ok(())
    }
}
