//! Motion: springs and curves, with values that can be interrupted.
//!
//! The rule the interface follows is that motion explains a change and never
//! delays one. That needs two things a CSS transition does not give:
//!
//! * **Interruption without a jump.** If a panel is sliding open and the person
//!   closes it, it must turn round from where it is, at the speed it has. A
//!   [`Spring`] keeps its velocity when its target changes, so it does.
//! * **A clock the window owns.** Nothing here reads the time. The window calls
//!   [`Spring::step`] with the frame's delta, which makes every animation
//!   reproducible in a test and pausable when the window is hidden.
//!
//! Durations are short on purpose (see [`duration`]): something done a hundred
//! times a day should not animate at all, and nothing takes longer than a third
//! of a second.

use std::time::Duration;

/// The few durations the interface uses.
pub mod duration {
    use std::time::Duration;

    /// Press feedback, hover, a checkbox.
    pub const INSTANT: Duration = Duration::from_millis(90);
    /// A popover, a menu, a tooltip.
    pub const FAST: Duration = Duration::from_millis(140);
    /// A panel, a sheet, a list reordering.
    pub const BASE: Duration = Duration::from_millis(200);
    /// A dialog or a full-screen change. Nothing is slower.
    pub const SLOW: Duration = Duration::from_millis(280);
}

/// An easing curve: progress in, eased progress out, both 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ease {
    /// Constant speed. For progress bars and nothing else.
    Linear,
    /// Fast, then settling. For anything entering or responding to a person.
    Out,
    /// Slow, fast, slow. For something moving between two places on screen.
    InOut,
    /// A cubic bezier, as CSS writes it.
    Bezier(f32, f32, f32, f32),
}

impl Ease {
    /// The eased value at `t`. `t` outside 0..=1 is clamped.
    pub fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            // The curve the interface uses for nearly everything: quick to
            // start, so the response is felt at once, and long to settle.
            Self::Out => bezier(0.23, 1.0, 0.32, 1.0, t),
            Self::InOut => bezier(0.77, 0.0, 0.175, 1.0, t),
            Self::Bezier(x1, y1, x2, y2) => bezier(x1, y1, x2, y2, t),
        }
    }
}

/// The y of a cubic bezier through (0,0), (x1,y1), (x2,y2), (1,1) at x.
fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let curve = |a: f32, b: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    };
    // Find the parameter whose x is the one asked for. The curve's x is
    // monotonic for control points in 0..=1, so bisection always converges.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if curve(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    curve(y1, y2, (lo + hi) / 2.0)
}

/// A timed change from 0 to 1 along a curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tween {
    /// How long it takes.
    pub duration: Duration,
    /// Its curve.
    pub ease: Ease,
    elapsed: Duration,
}

impl Tween {
    /// A tween that has not started.
    pub fn new(duration: Duration, ease: Ease) -> Self {
        Self {
            duration,
            ease,
            elapsed: Duration::ZERO,
        }
    }

    /// Advance by one frame and return the eased progress.
    pub fn step(&mut self, dt: Duration) -> f32 {
        self.elapsed = (self.elapsed + dt).min(self.duration);
        self.value()
    }

    /// The eased progress now.
    pub fn value(&self) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        self.ease
            .at(self.elapsed.as_secs_f32() / self.duration.as_secs_f32())
    }

    /// Whether it has reached the end.
    pub fn done(&self) -> bool {
        self.elapsed >= self.duration
    }
}

/// How a spring feels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringConfig {
    /// How hard it pulls toward the target. Higher is faster.
    pub stiffness: f32,
    /// How much it resists moving. At or above `2 * sqrt(stiffness)` it does not overshoot.
    pub damping: f32,
}

impl SpringConfig {
    /// Quick with no overshoot: panels, sheets, anything a person opens.
    pub const SNAPPY: Self = Self {
        stiffness: 520.0,
        damping: 46.0,
    };
    /// A little softer, still no overshoot: list rows finding their place.
    pub const SMOOTH: Self = Self {
        stiffness: 300.0,
        damping: 35.0,
    };
    /// A slight overshoot: something dropped, something that should feel physical.
    pub const BOUNCY: Self = Self {
        stiffness: 380.0,
        damping: 24.0,
    };

    /// A spring described the way a designer thinks of it: how long it takes to
    /// settle, and how much it bounces (0 is none, 1 never stops).
    pub fn from_duration(settle: Duration, bounce: f32) -> Self {
        let seconds = settle.as_secs_f32().max(0.01);
        let omega = std::f32::consts::TAU / seconds;
        let ratio = 1.0 - bounce.clamp(0.0, 0.95);
        Self {
            stiffness: omega * omega,
            damping: 2.0 * ratio * omega,
        }
    }
}

/// A value that moves toward a target like a mass on a spring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    /// Where it is.
    pub value: f32,
    /// Where it is going.
    pub target: f32,
    /// How fast it is moving, per second.
    pub velocity: f32,
    /// How it feels.
    pub config: SpringConfig,
}

/// Below these, a spring is close and slow enough to be drawn as arrived.
const REST_DISTANCE: f32 = 0.001;
const REST_SPEED: f32 = 0.01;
/// The largest slice of time integrated in one go. A dropped frame is cut into
/// slices this long, so a long pause cannot make the spring explode.
const MAX_SLICE: f32 = 1.0 / 240.0;

impl Spring {
    /// A spring at rest at `value`.
    pub fn at(value: f32, config: SpringConfig) -> Self {
        Self {
            value,
            target: value,
            velocity: 0.0,
            config,
        }
    }

    /// Change where it is going. It keeps moving as it was, so a change of mind
    /// mid-flight turns round smoothly.
    pub fn set(&mut self, target: f32) {
        self.target = target;
    }

    /// Put it somewhere with no animation.
    pub fn jump(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    /// Advance by one frame. Returns whether it is still moving.
    pub fn step(&mut self, dt: Duration) -> bool {
        let mut left = dt.as_secs_f32().min(0.25);
        while left > 0.0 {
            let h = left.min(MAX_SLICE);
            // Semi-implicit Euler: stable for a damped spring at this step size.
            let force = self.config.stiffness * (self.target - self.value)
                - self.config.damping * self.velocity;
            self.velocity += force * h;
            self.value += self.velocity * h;
            left -= h;
        }
        if self.resting() {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        true
    }

    /// Whether it has arrived.
    pub fn resting(&self) -> bool {
        (self.target - self.value).abs() < REST_DISTANCE && self.velocity.abs() < REST_SPEED
    }
}

/// Whether to move things or only fade them.
///
/// A person who asked the system for less motion still needs to see that
/// something changed. They get the fade and lose the travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preference {
    /// Movement and fades.
    #[default]
    Full,
    /// Fades only; positions change at once.
    Reduced,
}

impl Preference {
    /// A distance to travel, which is nothing when motion is reduced.
    pub fn travel(self, distance: f32) -> f32 {
        match self {
            Self::Full => distance,
            Self::Reduced => 0.0,
        }
    }
}

/// Where each of a list's rows should be drawn while it reorders.
///
/// Each row has a spring for its offset from its final place. When the order
/// changes, a row's spring starts at the distance it was displaced by and
/// relaxes to zero, so rows slide to their new positions instead of swapping.
#[derive(Debug, Clone, Default)]
pub struct Reorder {
    rows: Vec<(String, usize, Spring)>,
}

impl Reorder {
    /// A list with nothing in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Give the list's current order, as row ids. `row_height` is the distance
    /// between two rows.
    pub fn set_order(&mut self, ids: &[String], row_height: f32) {
        let mut next = Vec::with_capacity(ids.len());
        for (index, id) in ids.iter().enumerate() {
            let spring = match self.rows.iter().find(|(held, ..)| held == id) {
                Some((_, old_index, spring)) => {
                    let mut spring = *spring;
                    // Keep the row where it is on screen, then let it travel.
                    spring.value += (*old_index as f32 - index as f32) * row_height;
                    spring.set(0.0);
                    spring
                }
                // A new row has no previous place to travel from.
                None => Spring::at(0.0, SpringConfig::SMOOTH),
            };
            next.push((id.clone(), index, spring));
        }
        self.rows = next;
    }

    /// Advance every row. Returns whether any is still moving.
    pub fn step(&mut self, dt: Duration) -> bool {
        let mut moving = false;
        for (_, _, spring) in &mut self.rows {
            moving |= spring.step(dt);
        }
        moving
    }

    /// How far a row is from its final place right now.
    pub fn offset(&self, id: &str) -> f32 {
        self.rows
            .iter()
            .find(|(held, ..)| held == id)
            .map(|(_, _, s)| s.value)
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_micros(16_667);

    fn run(spring: &mut Spring, limit: usize) -> usize {
        (0..limit).take_while(|_| spring.step(FRAME)).count()
    }

    #[test]
    fn curves_start_at_zero_end_at_one_and_never_go_backwards() {
        for ease in [
            Ease::Linear,
            Ease::Out,
            Ease::InOut,
            Ease::Bezier(0.4, 0.0, 0.2, 1.0),
        ] {
            assert!(ease.at(0.0).abs() < 0.001, "{ease:?}");
            assert!((ease.at(1.0) - 1.0).abs() < 0.001, "{ease:?}");
            let mut last = 0.0;
            for n in 0..=100 {
                let v = ease.at(n as f32 / 100.0);
                assert!(v + 0.0005 >= last, "{ease:?} went backwards at {n}");
                last = v;
            }
        }
        assert!(
            Ease::Out.at(0.25) > 0.6,
            "ease-out does most of its work early"
        );
    }

    #[test]
    fn a_tween_ends_exactly_at_one() {
        let mut tween = Tween::new(duration::BASE, Ease::Out);
        let mut frames = 0;
        while !tween.done() {
            tween.step(FRAME);
            frames += 1;
        }
        assert_eq!(tween.value(), 1.0);
        assert_eq!(frames, 12, "200 ms is twelve frames at 60 Hz");
        assert_eq!(Tween::new(Duration::ZERO, Ease::Out).value(), 1.0);
    }

    #[test]
    fn a_snappy_spring_arrives_quickly_without_overshooting() {
        let mut spring = Spring::at(0.0, SpringConfig::SNAPPY);
        spring.set(1.0);
        let mut peak = 0.0f32;
        let mut frames = 0;
        while spring.step(FRAME) {
            peak = peak.max(spring.value);
            frames += 1;
            assert!(frames < 120, "never settled");
        }
        assert_eq!(spring.value, 1.0);
        assert!(peak <= 1.0005, "overshot to {peak}");
        assert!(
            frames <= 30,
            "took {frames} frames, more than half a second"
        );
    }

    #[test]
    fn a_bouncy_spring_overshoots_and_still_settles() {
        let mut spring = Spring::at(0.0, SpringConfig::BOUNCY);
        spring.set(1.0);
        let mut peak = 0.0f32;
        for _ in 0..240 {
            if !spring.step(FRAME) {
                break;
            }
            peak = peak.max(spring.value);
        }
        assert!(peak > 1.01 && peak < 1.25, "peak {peak}");
        assert!(spring.resting());
    }

    #[test]
    fn changing_the_target_mid_flight_keeps_the_velocity() {
        let mut spring = Spring::at(0.0, SpringConfig::SNAPPY);
        spring.set(1.0);
        for _ in 0..4 {
            spring.step(FRAME);
        }
        let (value, velocity) = (spring.value, spring.velocity);
        assert!(velocity > 0.0);
        spring.set(0.0);
        assert_eq!(
            (spring.value, spring.velocity),
            (value, velocity),
            "no jump, no stop"
        );
        spring.step(FRAME);
        assert!(
            spring.value > value - 0.2,
            "it eases round; it does not snap back"
        );
        run(&mut spring, 240);
        assert_eq!(spring.value, 0.0);
    }

    #[test]
    fn a_long_pause_does_not_make_the_spring_explode() {
        let mut spring = Spring::at(0.0, SpringConfig::SNAPPY);
        spring.set(100.0);
        spring.step(Duration::from_secs(5));
        assert!(spring.value.is_finite() && spring.value <= 101.0 && spring.value >= 0.0);
    }

    #[test]
    fn a_spring_can_be_described_by_how_long_it_takes() {
        let mut spring = Spring::at(
            0.0,
            SpringConfig::from_duration(Duration::from_millis(300), 0.0),
        );
        spring.set(1.0);
        let frames = run(&mut spring, 240);
        assert!((10..=40).contains(&frames), "{frames} frames");
    }

    #[test]
    fn reduced_motion_keeps_the_change_and_drops_the_travel() {
        assert_eq!(Preference::Full.travel(12.0), 12.0);
        assert_eq!(Preference::Reduced.travel(12.0), 0.0);
    }

    #[test]
    fn reordered_rows_slide_from_where_they_were() {
        let ids = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let mut list = Reorder::new();
        list.set_order(&ids(&["a", "b", "c"]), 40.0);
        assert_eq!(list.offset("a"), 0.0);
        list.set_order(&ids(&["c", "a", "b"]), 40.0);
        assert_eq!(
            list.offset("c"),
            80.0,
            "c was two rows down and is drawn there first"
        );
        assert_eq!(list.offset("a"), -40.0);
        while list.step(FRAME) {}
        assert_eq!(
            (list.offset("a"), list.offset("b"), list.offset("c")),
            (0.0, 0.0, 0.0)
        );
    }
}
