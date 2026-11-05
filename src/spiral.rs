//! Shared constant-pitch geometry. Writable limits do not define the spiral.
use crate::{codec::RAW, project::Disc};

pub(crate) struct Spiral {
    inner: f64,
    span: f64,
    sectors: f64,
    velocity: f64,
    angle: f64,
}

impl Spiral {
    pub(crate) fn new(disc: &Disc) -> Self {
        let (inner, outer, reference) = disc.geometry();
        Self {
            inner,
            span: outer.powi(2) - inner.powi(2),
            sectors: f64::from(reference),
            velocity: disc.velocity,
            angle: disc.angle.to_radians(),
        }
    }

    pub(crate) fn radius(&self, sector: f64) -> f64 {
        (self.inner.powi(2) + sector / self.sectors * self.span).sqrt()
    }

    pub(crate) fn sector(&self, radius: f64) -> f64 {
        (radius.powi(2) - self.inner.powi(2)) * self.sectors / self.span
    }

    pub(crate) fn angle(&self, radius: f64) -> f64 {
        self.angle + 2.0 * self.velocity * (self.sectors / 75.0) / self.span * (radius - self.inner)
    }

    pub(crate) fn point(&self, byte: i64) -> (f64, f64) {
        // Retain the byte-domain operation order: tiny phase changes matter
        // after thousands of turns and can change thresholded payload bytes.
        let fraction = byte.max(0) as f64 / (self.sectors * RAW as f64);
        let radius = (self.inner.powi(2) + fraction * self.span).sqrt();
        let (sin, cos) = self.angle(radius).sin_cos();
        (radius * cos, radius * sin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radial_inverse_and_byte_projection_share_the_reference() {
        let disc = Disc::default();
        let spiral = Spiral::new(&disc);
        for sector in [0, 11850, 120520, 359847] {
            let radius = spiral.radius(f64::from(sector));
            assert!((spiral.sector(radius) - f64::from(sector)).abs() < 1e-9);
            let (x, y) = spiral.point(i64::from(sector) * RAW as i64);
            assert!((x.hypot(y) - radius).abs() < 1e-12);
        }
    }
}
