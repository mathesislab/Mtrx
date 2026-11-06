//! Small vector icons drawn with a canvas (the text font has no such glyphs).

use std::f32::consts::FRAC_PI_2;

use iced::{
    Color, Element, Rectangle, Renderer, Theme, Vector, mouse,
    widget::canvas::{self, Frame, Geometry, Path, Stroke},
};

/// A clock face with its hands at twelve and four.
pub fn clock<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Clock { color })
        .width(size)
        .height(size)
        .into()
}

struct Clock {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Clock {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = (bounds.width / 9.0).max(1.0);
        let center = frame.center();
        let radius = bounds.width / 2.0 - width;
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width(width)
            .with_line_cap(canvas::LineCap::Round);
        frame.stroke(&Path::circle(center, radius), stroke);
        // Angles from 12 o'clock, clockwise.
        let hand = |angle: f32, length: f32| {
            let direction = Vector::new((angle - FRAC_PI_2).cos(), (angle - FRAC_PI_2).sin());
            Path::line(center, center + direction * (radius * length))
        };
        frame.stroke(&hand(0.0, 0.7), stroke);
        frame.stroke(&hand(FRAC_PI_2 * 4.0 / 3.0, 0.5), stroke);
        vec![frame.into_geometry()]
    }
}

/// A gear with eight teeth.
pub fn gear<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Gear { color })
        .width(size)
        .height(size)
        .into()
}

struct Gear {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Gear {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        const TEETH: usize = 8;
        let mut frame = Frame::new(renderer, bounds.size());
        let width = (bounds.width / 12.0).max(1.0);
        let center = frame.center();
        let outer = bounds.width / 2.0 - width;
        let inner = outer * 0.72;
        let point =
            |angle: f32, radius: f32| center + Vector::new(angle.cos(), angle.sin()) * radius;
        // Each tooth spans a quarter of its sector at the tip, a half at the root.
        let sector = std::f32::consts::TAU / TEETH as f32;
        let outline = Path::new(|path| {
            for i in 0..TEETH {
                let middle = i as f32 * sector;
                let corners = [
                    point(middle - sector / 4.0, inner),
                    point(middle - sector / 8.0, outer),
                    point(middle + sector / 8.0, outer),
                    point(middle + sector / 4.0, inner),
                ];
                if i == 0 {
                    path.move_to(corners[0]);
                } else {
                    path.line_to(corners[0]);
                }
                for corner in &corners[1..] {
                    path.line_to(*corner);
                }
            }
            path.close();
        });
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width(width)
            .with_line_join(canvas::LineJoin::Round);
        frame.stroke(&outline, stroke);
        frame.stroke(&Path::circle(center, outer * 0.3), stroke);
        vec![frame.into_geometry()]
    }
}

/// A video camera: a rounded body with a lens triangle on the right.
pub fn camera<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Camera { color })
        .width(size)
        .height(size)
        .into()
}

struct Camera {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Camera {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let color = (self.color)(theme);
        let body = Path::rounded_rectangle(
            iced::Point::new(0.0, h * 0.25),
            iced::Size::new(w * 0.68, h * 0.5),
            (h * 0.1).into(),
        );
        frame.fill(&body, color);
        let lens = Path::new(|path| {
            path.move_to(iced::Point::new(w * 0.72, h * 0.5));
            path.line_to(iced::Point::new(w, h * 0.28));
            path.line_to(iced::Point::new(w, h * 0.72));
            path.close();
        });
        frame.fill(&lens, color);
        vec![frame.into_geometry()]
    }
}

/// Two chain links across: a link to open.
pub fn link<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Link { color })
        .width(size)
        .height(size)
        .into()
}

struct Link {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Link {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width((s / 9.0).max(1.0));
        // Two rounded links overlapping along the diagonal.
        let (w, h) = (s * 0.5, s * 0.3);
        frame.translate(Vector::new(s / 2.0, s / 2.0));
        frame.rotate(-std::f32::consts::FRAC_PI_4);
        for x in [-s * 0.4, s * 0.4 - w] {
            let link = Path::rounded_rectangle(
                iced::Point::new(x, -h / 2.0),
                iced::Size::new(w, h),
                (h / 2.0).into(),
            );
            frame.stroke(&link, stroke);
        }
        vec![frame.into_geometry()]
    }
}

/// A pushpin, leaning right: a pinned message.
pub fn pin<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Pin { color })
        .width(size)
        .height(size)
        .into()
}

struct Pin {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Pin {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let color = (self.color)(theme);
        let rect = |x: f32, y: f32, w: f32, h: f32| {
            Path::rectangle(
                iced::Point::new(x * s, y * s),
                iced::Size::new(w * s, h * s),
            )
        };
        frame.translate(Vector::new(s / 2.0, s / 2.0));
        frame.rotate(std::f32::consts::FRAC_PI_4);
        // The cap, the barrel, the collar, then the needle.
        frame.fill(&rect(-0.2, -0.46, 0.4, 0.12), color);
        frame.fill(&rect(-0.12, -0.36, 0.24, 0.32), color);
        frame.fill(&rect(-0.28, -0.06, 0.56, 0.12), color);
        frame.stroke(
            &Path::line(
                iced::Point::new(0.0, 0.04 * s),
                iced::Point::new(0.0, 0.46 * s),
            ),
            Stroke::default()
                .with_color(color)
                .with_width((s / 10.0).max(1.0))
                .with_line_cap(canvas::LineCap::Round),
        );
        vec![frame.into_geometry()]
    }
}

/// A page with its top right corner folded.
pub fn file<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(File { color })
        .width(size)
        .height(size)
        .into()
}

struct File {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for File {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let width = (w / 10.0).max(1.0);
        let (left, right) = (w * 0.18 + width / 2.0, w * 0.82 - width / 2.0);
        let (top, bottom) = (width / 2.0, h - width / 2.0);
        let fold = (right - left) * 0.4;
        let point = iced::Point::new;
        let page = Path::new(|path| {
            path.move_to(point(left, top));
            path.line_to(point(right - fold, top));
            path.line_to(point(right, top + fold));
            path.line_to(point(right, bottom));
            path.line_to(point(left, bottom));
            path.close();
        });
        let corner = Path::new(|path| {
            path.move_to(point(right - fold, top));
            path.line_to(point(right - fold, top + fold));
            path.line_to(point(right, top + fold));
        });
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width(width)
            .with_line_join(canvas::LineJoin::Round);
        frame.stroke(&page, stroke);
        frame.stroke(&corner, stroke);
        vec![frame.into_geometry()]
    }
}

/// A paper clip, tilted.
pub fn paperclip<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Paperclip { color })
        .width(size)
        .height(size)
        .into()
}

struct Paperclip {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Paperclip {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        use std::f32::consts::PI;
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let at = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        // Three straight sides joined by half circles, from the inner end
        // outwards: down, round the bottom, up, round the top, down, round
        // the bottom, up.
        let half = |path: &mut canvas::path::Builder, x: f32, y: f32, r: f32, bottom: bool| {
            let (start, end) = if bottom { (0.0, PI) } else { (PI, 2.0 * PI) };
            path.arc(canvas::path::Arc {
                center: at(x, y),
                radius: r * s,
                start_angle: iced::Radians(start),
                end_angle: iced::Radians(end),
            });
        };
        let clip = Path::new(|path| {
            path.move_to(at(0.61, 0.32));
            path.line_to(at(0.61, 0.66));
            half(path, 0.5, 0.66, 0.11, true);
            path.line_to(at(0.39, 0.24));
            half(path, 0.535, 0.24, 0.145, false);
            path.line_to(at(0.68, 0.74));
            half(path, 0.5, 0.74, 0.18, true);
            path.line_to(at(0.32, 0.36));
        });
        let center = frame.center();
        frame.translate(Vector::new(center.x, center.y));
        frame.rotate(iced::Radians(PI / 4.0));
        frame.translate(Vector::new(-center.x, -center.y));
        frame.stroke(
            &clip,
            Stroke::default()
                .with_color((self.color)(theme))
                .with_width((s / 12.0).max(1.0))
                .with_line_cap(canvas::LineCap::Round),
        );
        vec![frame.into_geometry()]
    }
}

/// A smiling face, for choosing emoji.
pub fn smiley<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Smiley { color })
        .width(size)
        .height(size)
        .into()
}

struct Smiley {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Smiley {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        use std::f32::consts::PI;
        let mut frame = Frame::new(renderer, bounds.size());
        let color = (self.color)(theme);
        let width = (bounds.width / 12.0).max(1.0);
        let center = frame.center();
        let radius = bounds.width / 2.0 - width;
        let stroke = Stroke::default()
            .with_color(color)
            .with_width(width)
            .with_line_cap(canvas::LineCap::Round);
        frame.stroke(&Path::circle(center, radius), stroke);
        for side in [-1.0, 1.0] {
            let eye = center + Vector::new(side * radius * 0.35, -radius * 0.25);
            frame.fill(&Path::circle(eye, width * 0.9), color);
        }
        let smile = Path::new(|path| {
            path.arc(canvas::path::Arc {
                center,
                radius: radius * 0.55,
                start_angle: iced::Radians(PI * 0.2),
                end_angle: iced::Radians(PI * 0.8),
            });
        });
        frame.stroke(&smile, stroke);
        vec![frame.into_geometry()]
    }
}

/// A five-pointed star: filled in gold, or an outline in `color`.
pub fn star<'a, M: 'a>(size: f32, filled: bool, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Star { filled, color })
        .width(size)
        .height(size)
        .into()
}

/// The filled star's colour.
pub const GOLD: Color = Color::from_rgb(0.96, 0.72, 0.05);

struct Star {
    filled: bool,
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Star {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = (bounds.width / 12.0).max(1.0);
        let outer = bounds.width / 2.0 - width;
        let inner = outer * 0.45;
        // A little below the middle: the star's points reach higher than
        // its base.
        let center = frame.center() + Vector::new(0.0, outer * 0.08);
        let star = Path::new(|path| {
            for i in 0..10 {
                let radius = if i % 2 == 0 { outer } else { inner };
                let angle = std::f32::consts::PI / 5.0 * i as f32 - FRAC_PI_2;
                let point = center + Vector::new(angle.cos(), angle.sin()) * radius;
                if i == 0 {
                    path.move_to(point);
                } else {
                    path.line_to(point);
                }
            }
            path.close();
        });
        if self.filled {
            frame.fill(&star, GOLD);
            frame.stroke(
                &star,
                Stroke::default()
                    .with_color(GOLD)
                    .with_width(width)
                    .with_line_join(canvas::LineJoin::Round),
            );
        } else {
            frame.stroke(
                &star,
                Stroke::default()
                    .with_color((self.color)(theme))
                    .with_width(width)
                    .with_line_join(canvas::LineJoin::Round),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// An arrow curving back to the left: reply.
pub fn reply<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Reply { color })
        .width(size)
        .height(size)
        .into()
}

struct Reply {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Reply {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let width = (s / 10.0).max(1.0);
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width(width)
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        // Head pointing left, shaft curving down to the right.
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.42, 0.2));
                path.line_to(p(0.15, 0.45));
                path.line_to(p(0.42, 0.7));
            }),
            stroke,
        );
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.15, 0.45));
                path.line_to(p(0.55, 0.45));
                path.quadratic_curve_to(p(0.85, 0.45), p(0.85, 0.82));
            }),
            stroke,
        );
        vec![frame.into_geometry()]
    }
}

/// A pencil: edit.
pub fn pencil<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Pencil { color })
        .width(size)
        .height(size)
        .into()
}

struct Pencil {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Pencil {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let width = (s / 10.0).max(1.0);
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width(width)
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        // A slanted body with a pointed tip at the bottom left.
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.65, 0.15));
                path.line_to(p(0.85, 0.35));
                path.line_to(p(0.35, 0.85));
                path.line_to(p(0.13, 0.87));
                path.line_to(p(0.15, 0.65));
                path.close();
            }),
            stroke,
        );
        frame.stroke(&Path::line(p(0.55, 0.25), p(0.75, 0.45)), stroke);
        vec![frame.into_geometry()]
    }
}

/// A bin: delete.
pub fn trash<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Trash { color })
        .width(size)
        .height(size)
        .into()
}

struct Trash {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Trash {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let width = (s / 10.0).max(1.0);
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width(width)
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        // Lid with a handle, then the tapering bin with two lines.
        frame.stroke(&Path::line(p(0.15, 0.27), p(0.85, 0.27)), stroke);
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.38, 0.27));
                path.line_to(p(0.38, 0.14));
                path.line_to(p(0.62, 0.14));
                path.line_to(p(0.62, 0.27));
            }),
            stroke,
        );
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.24, 0.27));
                path.line_to(p(0.3, 0.88));
                path.line_to(p(0.7, 0.88));
                path.line_to(p(0.76, 0.27));
            }),
            stroke,
        );
        frame.stroke(&Path::line(p(0.43, 0.42), p(0.44, 0.74)), stroke);
        frame.stroke(&Path::line(p(0.57, 0.42), p(0.56, 0.74)), stroke);
        vec![frame.into_geometry()]
    }
}

/// An arrow pointing up (`true`) or down.
pub fn arrow<'a, M: 'a>(size: f32, up: bool, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Arrow { up, color })
        .width(size)
        .height(size)
        .into()
}

struct Arrow {
    up: bool,
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Arrow {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width((s / 8.0).max(1.0))
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        // Drawn pointing down, flipped for up.
        let p = |x: f32, y: f32| {
            let y = if self.up { 1.0 - y } else { y };
            iced::Point::new(x * s, y * s)
        };
        frame.stroke(&Path::line(p(0.5, 0.15), p(0.5, 0.85)), stroke);
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.2, 0.55));
                path.line_to(p(0.5, 0.85));
                path.line_to(p(0.8, 0.55));
            }),
            stroke,
        );
        vec![frame.into_geometry()]
    }
}

/// A chevron pointing down (`open`) or right: a section expanded or
/// collapsed.
pub fn chevron<'a, M: 'a>(size: f32, open: bool, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Chevron { open, color })
        .width(size)
        .height(size)
        .into()
}

struct Chevron {
    open: bool,
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Chevron {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width((s / 7.0).max(1.0))
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        // Drawn pointing down, turned for right.
        let p = |x: f32, y: f32| {
            let (x, y) = if self.open { (x, y) } else { (y, 1.0 - x) };
            iced::Point::new(x * s, y * s)
        };
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.2, 0.35));
                path.line_to(p(0.5, 0.65));
                path.line_to(p(0.8, 0.35));
            }),
            stroke,
        );
        vec![frame.into_geometry()]
    }
}

/// Two columns of three dots: something to drag.
pub fn grip<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Grip { color })
        .width(size)
        .height(size)
        .into()
}

struct Grip {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Grip {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let radius = (s / 12.0).max(0.75);
        for x in [0.38, 0.62] {
            for y in [0.25, 0.5, 0.75] {
                let dot = Path::circle(iced::Point::new(x * s, y * s), radius);
                frame.fill(&dot, (self.color)(theme));
            }
        }
        vec![frame.into_geometry()]
    }
}

/// An arrow down into a tray: download, save.
pub fn download<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Download { color })
        .width(size)
        .height(size)
        .into()
}

struct Download {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Download {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width((s / 10.0).max(1.0))
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        frame.stroke(&Path::line(p(0.5, 0.12), p(0.5, 0.62)), stroke);
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.28, 0.42));
                path.line_to(p(0.5, 0.64));
                path.line_to(p(0.72, 0.42));
            }),
            stroke,
        );
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.15, 0.66));
                path.line_to(p(0.15, 0.86));
                path.line_to(p(0.85, 0.86));
                path.line_to(p(0.85, 0.66));
            }),
            stroke,
        );
        vec![frame.into_geometry()]
    }
}

/// A padlock: private.
pub fn lock<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Lock { color })
        .width(size)
        .height(size)
        .into()
}

struct Lock {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Lock {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let color = (self.color)(theme);
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        // The shackle, then the body.
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.32, 0.48));
                path.line_to(p(0.32, 0.34));
                path.arc(canvas::path::Arc {
                    center: p(0.5, 0.34),
                    radius: 0.18 * s,
                    start_angle: iced::Radians(std::f32::consts::PI),
                    end_angle: iced::Radians(2.0 * std::f32::consts::PI),
                });
                path.line_to(p(0.68, 0.48));
            }),
            Stroke::default()
                .with_color(color)
                .with_width((s / 10.0).max(1.0))
                .with_line_cap(canvas::LineCap::Round),
        );
        frame.fill(
            &Path::rounded_rectangle(
                p(0.2, 0.46),
                iced::Size::new(0.6 * s, 0.42 * s),
                (0.08 * s).into(),
            ),
            color,
        );
        vec![frame.into_geometry()]
    }
}

/// A notebook: notes to self.
pub fn notebook<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Notebook { color })
        .width(size)
        .height(size)
        .into()
}

struct Notebook {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Notebook {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width((s / 11.0).max(1.0))
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        // The cover, its spine, and lines of writing.
        frame.stroke(
            &Path::rounded_rectangle(
                p(0.2, 0.1),
                iced::Size::new(0.62 * s, 0.8 * s),
                (0.08 * s).into(),
            ),
            stroke,
        );
        frame.stroke(&Path::line(p(0.34, 0.1), p(0.34, 0.9)), stroke);
        for y in [0.32, 0.5, 0.68] {
            frame.stroke(&Path::line(p(0.47, y), p(0.7, y)), stroke);
        }
        vec![frame.into_geometry()]
    }
}

/// A paper plane flying right: send.
pub fn send<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(Send { color })
        .width(size)
        .height(size)
        .into()
}

struct Send {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for Send {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        let color = (self.color)(theme);
        // The wings, notched at the back, and the fold along the middle.
        frame.fill(
            &Path::new(|path| {
                path.move_to(p(0.1, 0.13));
                path.line_to(p(0.92, 0.5));
                path.line_to(p(0.1, 0.87));
                path.line_to(p(0.24, 0.5));
                path.close();
            }),
            color,
        );
        frame.stroke(
            &Path::line(p(0.24, 0.5), p(0.55, 0.5)),
            Stroke::default()
                .with_color(Color::BLACK.scale_alpha(0.25))
                .with_width((s / 14.0).max(1.0)),
        );
        vec![frame.into_geometry()]
    }
}

/// A tick in a circle: sent.
pub fn check_circle<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(CheckCircle { color })
        .width(size)
        .height(size)
        .into()
}

struct CheckCircle {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for CheckCircle {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let width = (s / 10.0).max(1.0);
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width(width)
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        frame.stroke(&Path::circle(frame.center(), s / 2.0 - width), stroke);
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.31, 0.52));
                path.line_to(p(0.45, 0.66));
                path.line_to(p(0.7, 0.38));
            }),
            stroke,
        );
        vec![frame.into_geometry()]
    }
}

/// A cross in a circle: not sent.
pub fn x_circle<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    canvas::Canvas::new(XCircle { color })
        .width(size)
        .height(size)
        .into()
}

struct XCircle {
    color: fn(&Theme) -> Color,
}

impl<M> canvas::Program<M> for XCircle {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let width = (s / 10.0).max(1.0);
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width(width)
            .with_line_cap(canvas::LineCap::Round);
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        frame.stroke(&Path::circle(frame.center(), s / 2.0 - width), stroke);
        frame.stroke(&Path::line(p(0.36, 0.36), p(0.64, 0.64)), stroke);
        frame.stroke(&Path::line(p(0.64, 0.36), p(0.36, 0.64)), stroke);
        vec![frame.into_geometry()]
    }
}

/// A cross: closes.
pub fn cross<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    sketch(size, color, |frame, s, stroke| {
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        frame.stroke(&Path::line(p(0.22, 0.22), p(0.78, 0.78)), stroke);
        frame.stroke(&Path::line(p(0.78, 0.22), p(0.22, 0.78)), stroke);
    })
}

/// A tick: done.
pub fn check<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    sketch(size, color, |frame, s, stroke| {
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.18, 0.54));
                path.line_to(p(0.4, 0.75));
                path.line_to(p(0.82, 0.27));
            }),
            stroke,
        );
    })
}

/// Two overlapping sheets: copies.
pub fn copy<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    sketch(size, color, |frame, s, stroke| {
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        let sheet = |x: f32, y: f32| {
            Path::rounded_rectangle(
                p(x, y),
                iced::Size::new(0.5 * s, 0.56 * s),
                (0.08 * s).into(),
            )
        };
        frame.stroke(&sheet(0.36, 0.3), stroke);
        // The sheet behind shows above and left of the front one.
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.24, 0.7));
                path.line_to(p(0.22, 0.7));
                path.line_to(p(0.14, 0.62));
                path.line_to(p(0.14, 0.22));
                path.line_to(p(0.22, 0.14));
                path.line_to(p(0.56, 0.14));
                path.line_to(p(0.64, 0.22));
            }),
            stroke,
        );
    })
}

/// A hash: anyone can join, as rooms are marked in the room list.
pub fn hash<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    iced::widget::text("#")
        .size(size)
        .font(crate::fonts::SEMIBOLD)
        .style(move |theme: &Theme| iced::widget::text::Style {
            color: Some(color(theme)),
        })
        .into()
}

/// Two people: members of a space can join.
pub fn people<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    sketch(size, color, |frame, s, stroke| {
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        let person = |frame: &mut Frame, x: f32| {
            frame.stroke(&Path::circle(p(x, 0.34), 0.13 * s), stroke);
            frame.stroke(
                &Path::new(|path| {
                    path.arc(canvas::path::Arc {
                        center: p(x, 0.82),
                        radius: 0.22 * s,
                        start_angle: iced::Radians(std::f32::consts::PI),
                        end_angle: iced::Radians(std::f32::consts::TAU),
                    });
                }),
                stroke,
            );
        };
        person(frame, 0.34);
        person(frame, 0.68);
    })
}

/// A door ajar: ask to join.
pub fn door<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    sketch(size, color, |frame, s, stroke| {
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.2, 0.86));
                path.line_to(p(0.2, 0.14));
                path.line_to(p(0.72, 0.14));
                path.line_to(p(0.72, 0.86));
            }),
            stroke,
        );
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.2, 0.14));
                path.line_to(p(0.5, 0.24));
                path.line_to(p(0.5, 0.92));
                path.line_to(p(0.2, 0.86));
            }),
            stroke,
        );
    })
}

/// A folder: where a file is.
pub fn folder<'a, M: 'a>(size: f32, color: fn(&Theme) -> Color) -> Element<'a, M> {
    sketch(size, color, |frame, s, stroke| {
        let p = |x: f32, y: f32| iced::Point::new(x * s, y * s);
        frame.stroke(
            &Path::new(|path| {
                path.move_to(p(0.12, 0.8));
                path.line_to(p(0.12, 0.22));
                path.line_to(p(0.4, 0.22));
                path.line_to(p(0.5, 0.32));
                path.line_to(p(0.88, 0.32));
                path.line_to(p(0.88, 0.8));
                path.close();
            }),
            stroke,
        );
    })
}

/// A ring filled clockwise from the top to `fraction`, as progress; just
/// the faint ring while it is unknown.
pub fn ring<'a, M: 'a>(size: f32, fraction: Option<f32>) -> Element<'a, M> {
    canvas::Canvas::new(Ring { fraction })
        .width(size)
        .height(size)
        .into()
}

struct Ring {
    fraction: Option<f32>,
}

impl<M> canvas::Program<M> for Ring {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = 2.0;
        let center = frame.center();
        let radius = bounds.width.min(bounds.height) / 2.0 - width;
        let palette = theme.extended_palette();
        let track = Stroke::default()
            .with_color(palette.background.strong.color)
            .with_width(width);
        frame.stroke(&Path::circle(center, radius), track);
        if let Some(fraction) = self.fraction.filter(|f| *f > 0.0) {
            let start = -FRAC_PI_2;
            let arc = Path::new(|path| {
                path.arc(canvas::path::Arc {
                    center,
                    radius,
                    start_angle: iced::Radians(start),
                    end_angle: iced::Radians(start + std::f32::consts::TAU * fraction.min(1.0)),
                });
            });
            let done = Stroke::default()
                .with_color(palette.primary.base.color)
                .with_width(width)
                .with_line_cap(canvas::LineCap::Round);
            frame.stroke(&arc, done);
        }
        vec![frame.into_geometry()]
    }
}

/// An icon of round-capped strokes, drawn by `draw` on a square of side
/// `s` (the size).
fn sketch<'a, M: 'a>(
    size: f32,
    color: fn(&Theme) -> Color,
    draw: fn(&mut Frame, f32, Stroke<'static>),
) -> Element<'a, M> {
    canvas::Canvas::new(Sketch { color, draw })
        .width(size)
        .height(size)
        .into()
}

struct Sketch {
    color: fn(&Theme) -> Color,
    draw: fn(&mut Frame, f32, Stroke<'static>),
}

impl<M> canvas::Program<M> for Sketch {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width;
        let stroke = Stroke::default()
            .with_color((self.color)(theme))
            .with_width((s / 11.0).max(1.0))
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        (self.draw)(&mut frame, s, stroke);
        vec![frame.into_geometry()]
    }
}
