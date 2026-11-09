//! Two-finger scrolls from the phone's touchpad, shaped as a laptop touchpad's are where the desktop
//! takes them as fingers on a touchpad (GNOME): locked to the way they mostly go. A touchpad's driver
//! does that, so scrolling down sends no sideways wobble; without it GNOME's app grid, which pages
//! on sideways scrolls, swung back and forth under a scroll down. A clear turn the other way moves
//! the lock over, as libinput's does.

/// Scroll units before the way is decided (a few pixels of finger travel).
const DECIDE: f64 = 3.0;
/// How much of the last moves' travel each new move keeps, for which way they go now.
const KEEP: f64 = 0.8;
/// The other way must lead by this much to take over.
const TURN: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Axis {
    X,
    Y,
}

#[derive(Default)]
pub struct ScrollLock {
    axis: Option<Axis>,
    /// Held until the way is decided.
    pending: (f64, f64),
    /// Recent travel each way, fading.
    recent: (f64, f64),
}

impl ScrollLock {
    /// One move: what to scroll, if anything yet.
    pub fn feed(&mut self, dx: f64, dy: f64) -> Option<(f64, f64)> {
        self.recent = (self.recent.0 * KEEP + dx.abs(), self.recent.1 * KEEP + dy.abs());
        let (dx, dy) = match self.axis {
            None => {
                self.pending = (self.pending.0 + dx, self.pending.1 + dy);
                let (px, py) = self.pending;
                if px.abs().max(py.abs()) < DECIDE {
                    return None;
                }
                self.axis = Some(if px.abs() > py.abs() { Axis::X } else { Axis::Y });
                self.pending = (0.0, 0.0);
                (px, py)
            }
            Some(Axis::Y) if self.recent.0 > TURN * self.recent.1 => {
                self.axis = Some(Axis::X);
                (dx, dy)
            }
            Some(Axis::X) if self.recent.1 > TURN * self.recent.0 => {
                self.axis = Some(Axis::Y);
                (dx, dy)
            }
            Some(_) => (dx, dy),
        };
        let out = match self.axis {
            Some(Axis::X) => (dx, 0.0),
            _ => (0.0, dy),
        };
        (out != (0.0, 0.0)).then_some(out)
    }

    /// The fingers lifted: the next scroll decides afresh.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_to_the_way_it_goes() {
        let mut s = ScrollLock::default();
        // A scroll down with a sideways wobble: held until clear, then down only.
        assert_eq!(s.feed(0.4, 1.0), None);
        assert_eq!(s.feed(-0.3, 1.2), None);
        assert_eq!(s.feed(0.2, 1.1), Some((0.0, 3.3000000000000003)));
        assert_eq!(s.feed(0.5, 1.0), Some((0.0, 1.0)));
        assert_eq!(s.feed(-0.6, 0.0), None);
        // A clear turn sideways takes the lock over.
        for _ in 0..6 {
            s.feed(2.0, 0.1);
        }
        assert_eq!(s.feed(2.0, 0.1), Some((2.0, 0.0)));
        s.reset();
        assert_eq!(s.feed(-4.0, 0.5), Some((-4.0, 0.0)));
    }
}
