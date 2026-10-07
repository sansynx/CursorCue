#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Following,
    Frozen,
    Hidden,
    Animating,
    Disabled,
}

pub struct Cursor {
    pub position: Point,
    pub real: Point,
    pub mode: Mode,
    animation: Option<(Point, f32, f32)>,
    previous: Mode,
    smoothing_rate: f32,
    resume_duration: Option<f32>,
}
impl Default for Cursor {
    fn default() -> Self {
        Self {
            position: Point::default(),
            real: Point::default(),
            mode: Mode::Following,
            animation: None,
            previous: Mode::Following,
            smoothing_rate: 0.0,
            resume_duration: None,
        }
    }
}
impl Cursor {
    pub fn set_motion(&mut self, smoothing_rate: f32, resume_duration: f32) {
        self.smoothing_rate = smoothing_rate.max(0.0);
        self.resume_duration = Some(resume_duration.clamp(0.12, 0.45));
    }
    pub fn track(&mut self, position: Option<Point>, elapsed: f32) {
        if let Some(point) = position {
            self.real = point;
        }
        match self.mode {
            Mode::Following => {
                if self.smoothing_rate == 0.0 {
                    self.position = self.real;
                } else {
                    let alpha = 1.0 - (-self.smoothing_rate * elapsed.max(0.0)).exp();
                    self.position.x += (self.real.x - self.position.x) * alpha;
                    self.position.y += (self.real.y - self.position.y) * alpha;
                }
            }
            Mode::Animating => {
                if let Some((start, time, duration)) = self.animation {
                    let time = time + elapsed.max(0.0);
                    let t = (time / duration).clamp(0.0, 1.0);
                    let eased = t * t * (3.0 - 2.0 * t);
                    self.position = Point {
                        x: start.x + (self.real.x - start.x) * eased,
                        y: start.y + (self.real.y - start.y) * eased,
                    };
                    self.animation = Some((start, time, duration));
                    if t >= 1.0 {
                        self.position = self.real;
                        self.mode = Mode::Following;
                        self.animation = None;
                    }
                }
            }
            _ => {}
        }
    }
    pub fn freeze(&mut self) {
        self.animation = None;
        self.mode = Mode::Frozen;
    }
    pub fn hide(&mut self) {
        if self.mode == Mode::Hidden {
            self.mode = self.previous;
        } else {
            self.previous = self.mode;
            self.mode = Mode::Hidden;
        }
        self.animation = None;
        if self.mode == Mode::Animating {
            self.mode = Mode::Frozen;
        }
    }
    pub fn resume(&mut self, smooth: bool) {
        if smooth {
            let distance = (self.position.x - self.real.x).hypot(self.position.y - self.real.y);
            self.animation = Some((
                self.position,
                0.0,
                self.resume_duration
                    .unwrap_or_else(|| (distance / 2400.0).clamp(0.12, 0.45)),
            ));
            self.mode = Mode::Animating;
        } else {
            self.position = self.real;
            self.mode = Mode::Following;
            self.animation = None;
        }
    }
    pub fn drop_here(&mut self) {
        self.position = self.real;
        self.freeze();
    }
    pub fn visible(&self) -> bool {
        !matches!(self.mode, Mode::Hidden | Mode::Disabled)
    }
}
pub fn map_to_source(point: Point, bounds: [f32; 4], size: Point) -> Option<Point> {
    let [left, top, right, bottom] = bounds;
    if right <= left
        || bottom <= top
        || size.x <= 0.0
        || size.y <= 0.0
        || point.x < left
        || point.x >= right
        || point.y < top
        || point.y >= bottom
    {
        return None;
    }
    Some(Point {
        x: (point.x - left) / (right - left) * size.x,
        y: (point.y - top) / (bottom - top) * size.y,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }
    #[test]
    fn smoothing_responds_without_snapping_and_converges_over_time() {
        let mut cursor = Cursor::default();
        cursor.set_motion(20.0, 0.2);
        cursor.track(Some(p(100.0, 50.0)), 0.016);
        assert!(cursor.position.x > 0.0 && cursor.position.x < 100.0);
        cursor.track(None, 0.5);
        assert!(cursor.position.x > 99.0);
    }
    #[test]
    fn configured_animation_finishes_at_its_selected_duration() {
        let mut cursor = Cursor::default();
        cursor.set_motion(0.0, 0.2);
        cursor.freeze();
        cursor.track(Some(p(900.0, 250.0)), 0.01);
        cursor.resume(true);
        cursor.track(None, 0.19);
        assert_eq!(cursor.mode, Mode::Animating);
        cursor.track(None, 0.011);
        assert_eq!(cursor.mode, Mode::Following);
        assert_eq!(cursor.position, p(900.0, 250.0));
    }
    #[test]
    fn frozen_cursor_stays_while_real_mouse_keeps_tracking() {
        let mut cursor = Cursor::default();
        cursor.track(Some(p(20.0, 40.0)), 0.016);
        cursor.freeze();
        cursor.track(Some(p(80.0, 90.0)), 0.016);
        assert_eq!(cursor.position, p(20.0, 40.0));
        assert_eq!(cursor.real, p(80.0, 90.0));
    }
    #[test]
    fn hide_is_immediate_and_drop_reveals_at_latest_real_position() {
        let mut cursor = Cursor::default();
        cursor.hide();
        assert!(!cursor.visible());
        cursor.track(Some(p(50.0, 60.0)), 0.01);
        cursor.drop_here();
        assert!(cursor.visible());
        assert_eq!(cursor.position, p(50.0, 60.0));
        assert_eq!(cursor.mode, Mode::Frozen);
    }
    #[test]
    fn smooth_resume_is_bounded_and_freeze_interrupts_animation() {
        let mut cursor = Cursor::default();
        cursor.freeze();
        cursor.track(Some(p(900.0, 250.0)), 0.01);
        cursor.resume(true);
        cursor.track(None, 0.06);
        assert!(cursor.position.x > 0.0 && cursor.position.x < 900.0);
        let rendered = cursor.position;
        cursor.freeze();
        cursor.track(Some(p(1000.0, 300.0)), 1.0);
        assert_eq!(cursor.position, rendered);
        cursor.resume(true);
        cursor.track(None, 0.5);
        assert_eq!(cursor.position, p(1000.0, 300.0));
        assert_eq!(cursor.mode, Mode::Following);
    }
    #[test]
    fn outside_source_retains_last_valid_audience_and_real_positions() {
        let mut cursor = Cursor::default();
        cursor.track(Some(p(30.0, 40.0)), 0.01);
        cursor.track(None, 0.1);
        assert_eq!(cursor.position, p(30.0, 40.0));
    }
    #[test]
    fn negative_monitor_coordinates_map_using_physical_bounds() {
        assert_eq!(
            map_to_source(
                p(-900.0, 300.0),
                [-1000.0, 200.0, -600.0, 600.0],
                p(800.0, 800.0)
            ),
            Some(p(200.0, 200.0))
        );
        assert_eq!(
            map_to_source(
                p(-1001.0, 300.0),
                [-1000.0, 200.0, -600.0, 600.0],
                p(800.0, 800.0)
            ),
            None
        );
    }
}
