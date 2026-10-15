//! Units of measure, edition 0.3 (ADR 0104): the normal form of a unit.
//!
//! A unit is a positive rational scale and the integer exponents of base units. A base unit is
//! identified by its declaration, the module that declares it and its name, so two modules'
//! `Meter`s are two dimensions (ADR 0104 rule 1). Two units are equal exactly when their
//! normal forms are (rule 2).

use std::collections::BTreeMap;
use std::fmt;

/// A base unit: its declaring module's path and its name (ADR 0104 rule 1).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct BaseUnit {
    pub(crate) module: String,
    pub(crate) name: String,
}

/// The normal form of a unit (ADR 0104 rule 2): the scale `num / den`, reduced and positive,
/// and the exponents of base units, none zero, ordered by module path and then name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Unit {
    num: u128,
    den: u128,
    dims: BTreeMap<BaseUnit, i32>,
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Unit {
    /// No unit: scale 1 and no base unit.
    pub(crate) fn one() -> Unit {
        Unit {
            num: 1,
            den: 1,
            dims: BTreeMap::new(),
        }
    }

    /// The base unit `base`.
    pub(crate) fn base(base: BaseUnit) -> Unit {
        Unit {
            num: 1,
            den: 1,
            dims: BTreeMap::from([(base, 1)]),
        }
    }

    /// The factor `n`, a scale with no base unit; `None` for zero.
    pub(crate) fn factor(n: u128) -> Option<Unit> {
        (n > 0).then(|| Unit {
            num: n,
            den: 1,
            dims: BTreeMap::new(),
        })
    }

    /// The scale `num / den`, reduced; `None` when it overflows.
    fn scaled(num: u128, den: u128, dims: BTreeMap<BaseUnit, i32>) -> Option<Unit> {
        let g = gcd(num, den);
        (g != 0).then(|| Unit {
            num: num / g,
            den: den / g,
            dims,
        })
    }

    /// The product of two units; `None` when the scale overflows `u128`.
    pub(crate) fn mul(&self, other: &Unit) -> Option<Unit> {
        let mut dims = self.dims.clone();
        for (base, exponent) in &other.dims {
            let entry = dims.entry(base.clone()).or_insert(0);
            *entry = entry.checked_add(*exponent)?;
            if *entry == 0 {
                dims.remove(base);
            }
        }
        // Cross-reduce first, so that the products stay as small as they can.
        let g1 = gcd(self.num, other.den);
        let g2 = gcd(other.num, self.den);
        let num = (self.num / g1).checked_mul(other.num / g2)?;
        let den = (self.den / g2).checked_mul(other.den / g1)?;
        Unit::scaled(num, den, dims)
    }

    /// The quotient of two units; `None` when the scale overflows `u128`.
    pub(crate) fn div(&self, other: &Unit) -> Option<Unit> {
        let inverse = Unit {
            num: other.den,
            den: other.num,
            dims: other.dims.iter().map(|(b, e)| (b.clone(), -e)).collect(),
        };
        self.mul(&inverse)
    }

    /// Whether this is no unit: scale 1 and no base unit (ADR 0104 rule 2).
    pub(crate) fn is_one(&self) -> bool {
        self.dims.is_empty() && self.num == 1 && self.den == 1
    }

    /// Whether it has no base unit, whatever its scale.
    #[cfg(test)]
    pub(crate) fn is_dimensionless(&self) -> bool {
        self.dims.is_empty()
    }

    /// The positive integer `k` such that a magnitude in `self` is `k` times the magnitude in
    /// `target`, when both have the same dimension and the ratio of their scales, source over
    /// target, is an integer (ADR 0104 rule 4).
    pub(crate) fn ratio_to(&self, target: &Unit) -> Option<u128> {
        if self.dims != target.dims {
            return None;
        }
        let num = self.num.checked_mul(target.den)?;
        let den = self.den.checked_mul(target.num)?;
        (den != 0 && num % den == 0).then(|| num / den)
    }

    /// Whether `self` and `other` have the same base units with the same exponents.
    pub(crate) fn same_dimension(&self, other: &Unit) -> bool {
        self.dims == other.dims
    }

    /// The unit written with module-qualified base units, to tell apart two that share a name.
    pub(crate) fn qualified(&self) -> String {
        self.written(true)
    }

    fn written(&self, qualified: bool) -> String {
        let name = |base: &BaseUnit| {
            if qualified {
                format!("{}.{}", base.module, base.name)
            } else {
                base.name.clone()
            }
        };
        let mut over: Vec<String> = Vec::new();
        let mut under: Vec<String> = Vec::new();
        if self.num != 1 {
            over.push(self.num.to_string());
        }
        if self.den != 1 {
            under.push(self.den.to_string());
        }
        for (base, exponent) in &self.dims {
            let side = if *exponent > 0 { &mut over } else { &mut under };
            for _ in 0..exponent.unsigned_abs() {
                side.push(name(base));
            }
        }
        let over = if over.is_empty() {
            "1".to_owned()
        } else {
            over.join(" * ")
        };
        match under.len() {
            0 => over,
            1 => format!("{over} / {}", under.first().map_or("", String::as_str)),
            _ => format!("{over} / ({})", under.join(" * ")),
        }
    }
}

/// A unit holds no type variable and no label (ADR 0077's flags).
impl super::shared::Flagged for Unit {
    fn flags(&self) -> super::shared::Flags {
        super::shared::Flags::LEAF
    }
}

/// `Meter / Second`, `1000 * Meter`, `Kilogram * Meter / (Second * Second)`.
impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.written(false))
    }
}

#[cfg(test)]
mod tests {
    use super::{BaseUnit, Unit};

    fn base(module: &str, name: &str) -> Unit {
        Unit::base(BaseUnit {
            module: module.to_owned(),
            name: name.to_owned(),
        })
    }

    #[test]
    fn units_compare_by_their_normal_form() {
        let meter = base("m", "Meter");
        let second = base("m", "Second");
        let km = Unit::factor(1000).unwrap().mul(&meter).unwrap();
        assert_eq!(km, meter.mul(&Unit::factor(1000).unwrap()).unwrap());
        assert_ne!(km, meter);
        // Exponents cancel; a dimensionless scale-1 unit is no unit.
        assert!(meter.div(&meter).unwrap().is_one());
        let speed = meter.div(&second).unwrap();
        assert_eq!(speed.mul(&second).unwrap(), meter);
        // Two modules' `Meter`s are two dimensions (ADR 0104 rule 1).
        assert_ne!(base("a", "Meter"), base("b", "Meter"));
        assert_eq!(base("a", "Meter").qualified(), "a.Meter");
    }

    #[test]
    fn conversions_go_by_an_integer_ratio_only() {
        let meter = base("m", "Meter");
        let km = Unit::factor(1000).unwrap().mul(&meter).unwrap();
        assert_eq!(km.ratio_to(&meter), Some(1000));
        assert_eq!(meter.ratio_to(&km), None);
        assert_eq!(meter.ratio_to(&meter), Some(1));
        assert_eq!(meter.ratio_to(&base("m", "Second")), None);
        // `Kilometer / Meter` is a scaled dimensionless unit: 1000 to no unit.
        let scaled = km.div(&meter).unwrap();
        assert!(scaled.is_dimensionless() && !scaled.is_one());
        assert_eq!(scaled.ratio_to(&Unit::one()), Some(1000));
        // `Second / 1000` has a rational scale.
        let ms = base("m", "Second")
            .div(&Unit::factor(1000).unwrap())
            .unwrap();
        assert_eq!(base("m", "Second").ratio_to(&ms), Some(1000));
    }

    #[test]
    fn units_are_written_with_their_scale() {
        let meter = base("m", "Meter");
        let second = base("m", "Second");
        let kg = base("m", "Kilogram");
        assert_eq!(meter.div(&second).unwrap().to_string(), "Meter / Second");
        let newton = kg
            .mul(&meter)
            .unwrap()
            .div(&second.mul(&second).unwrap())
            .unwrap();
        assert_eq!(newton.to_string(), "Kilogram * Meter / (Second * Second)");
        assert_eq!(
            Unit::factor(1000).unwrap().mul(&meter).unwrap().to_string(),
            "1000 * Meter"
        );
    }

    #[test]
    fn a_scale_that_overflows_is_refused() {
        let big = Unit::factor(u128::MAX).unwrap();
        assert_eq!(big.mul(&Unit::factor(2).unwrap()), None);
        assert!(Unit::factor(0).is_none());
    }
}
