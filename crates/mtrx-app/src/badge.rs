//! The unread count on the app's icon: the Dock on macOS, an overlay on the
//! taskbar button on Windows. Linux has no standard dock badge, so none.

use iced::Task;
use mtrx_core::{RoomKind, RoomSummary};

/// The count shown, to change it only when it changes.
#[derive(Default)]
pub struct Badge {
    shown: u64,
}

impl Badge {
    /// Show `count`; none for 0.
    pub fn show<T: Send + 'static>(&mut self, count: u64) -> Task<T> {
        if count == self.shown {
            return Task::none();
        }
        self.shown = count;
        set(count)
    }
}

/// Messages that notify by the account's Matrix notification settings,
/// in all rooms (muted ones have none).
pub fn count(rooms: &[RoomSummary]) -> u64 {
    rooms
        .iter()
        .filter(|r| r.kind != RoomKind::Space)
        .map(|r| r.unread_notifications)
        .sum()
}

#[cfg(target_os = "macos")]
fn set<T>(count: u64) -> Task<T> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    // AppKit only on the main thread, where iced updates the app (not in
    // tests).
    if let Some(main_thread) = MainThreadMarker::new() {
        let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
        NSApplication::sharedApplication(main_thread)
            .dockTile()
            .setBadgeLabel(label.as_deref());
    }
    Task::none()
}

/// On the window's taskbar button, which the runtime hands over on the main
/// thread.
#[cfg(windows)]
fn set<T: Send + 'static>(count: u64) -> Task<T> {
    use iced::window;

    window::oldest().then(move |id| match id {
        Some(id) => window::run(id, move |window| {
            crate::windows::taskbar_badge(window, count);
        })
        .discard(),
        None => Task::none(),
    })
}

/// Linux has no standard dock badge.
#[cfg(not(any(target_os = "macos", windows)))]
fn set<T>(_count: u64) -> Task<T> {
    Task::none()
}

/// The count as drawn on the badge: up to two digits.
#[cfg(any(windows, test))]
fn label(count: u64) -> String {
    if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    }
}

/// The badge for `count`, `size` pixels square, as straight RGBA: the count
/// in white on a red disc, like the room list's highlighted counts.
#[cfg(any(windows, test))]
pub fn image(count: u64, size: u32) -> Option<Vec<u8>> {
    use std::sync::Arc;

    use iced::Theme;
    use resvg::usvg;

    let label = label(count);
    let font_size = match label.len() {
        1 => 11.0,
        2 => 9.5,
        _ => 7.0,
    };
    // Digits are centred at about half Inter's cap height (0.73 em) above the
    // baseline.
    let baseline = 8.0 + 0.365 * font_size;
    let [r, g, b, _] = Theme::Light
        .extended_palette()
        .danger
        .base
        .color
        .into_rgba8();
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
<circle cx="8" cy="8" r="8" fill="#{r:02x}{g:02x}{b:02x}"/>
<text x="8" y="{baseline}" text-anchor="middle" font-family="{family}" font-weight="700" font-size="{font_size}" fill="#fff">{label}</text>
</svg>"##,
        family = crate::fonts::FAMILY,
    );
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_font_data(crate::fonts::BOLD.to_vec());
    let options = usvg::Options {
        fontdb: Arc::new(fonts),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(&svg, &options).ok()?;
    crate::logo::rasterize(&tree, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(kind: RoomKind, messages: u64, notifications: u64) -> RoomSummary {
        RoomSummary {
            id: "!room:example.org".try_into().unwrap(),
            name: String::new(),
            avatar: None,
            topic: None,
            kind,
            membership: mtrx_core::Membership::Joined,
            encrypted: false,
            join_rule: mtrx_core::JoinRule::Invite,
            unread_messages: messages,
            unread_mentions: 0,
            unread_notifications: notifications,
            last_activity: None,
            tag: mtrx_core::RoomTag::None,
            direct_user: None,
            alias: None,
            joined_members: 2,
            pinned: Vec::new(),
        }
    }

    /// Messages that notify count, in rooms and direct chats; other unread
    /// messages and spaces do not.
    #[test]
    fn counts_notifying_messages() {
        assert_eq!(count(&[]), 0);
        let rooms = [
            room(RoomKind::Room, 12, 2),
            room(RoomKind::Direct, 3, 3),
            room(RoomKind::Room, 40, 0),
            room(RoomKind::Space, 5, 5),
        ];
        assert_eq!(count(&rooms), 5);
    }

    /// Counts past two digits shorten.
    #[test]
    fn labels() {
        assert_eq!(label(7), "7");
        assert_eq!(label(42), "42");
        assert_eq!(label(100), "99+");
    }

    /// A red disc with the count in white, transparent around it.
    #[test]
    fn draws_disc_and_count() {
        let size = 32;
        let pixels = image(5, size).expect("rendered");
        assert_eq!(pixels.len(), (size * size * 4) as usize);
        let pixel = |x: u32, y: u32| {
            let i = ((y * size + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };
        assert_eq!(pixel(0, 0)[3], 0, "corner outside the disc");
        let [r, g, b, a] = pixel(3, size / 2);
        assert!(a == 255 && r > 150 && g < 100 && b < 100, "red disc edge");
        let white = (0..size)
            .flat_map(|y| (0..size).map(move |x| (x, y)))
            .filter(|&(x, y)| pixel(x, y).iter().all(|&c| c > 230))
            .count();
        assert!(white > 10, "count drawn ({white} white pixels)");
        assert_ne!(image(8, size), Some(pixels), "counts differ");
    }
}
