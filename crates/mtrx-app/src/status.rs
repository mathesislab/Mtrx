//! The status pill: what the open room waits on, or what went wrong, in one
//! place, floating at the timeline's bottom left above the composer, so
//! nothing moves. Light blue for a wait, light pink for a problem. Being
//! offline, or signed out, keeps its bar along the window's bottom: little
//! works then.

use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local};
use iced::{
    Alignment, Element,
    widget::{button, container, row, text},
};

use mtrx_core::OwnedRoomId;

use crate::{
    app,
    hint::{Position, hint},
    room::Message,
    style,
};

/// What the pill says.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Something failed, until dismissed.
    Problem(String),
    /// Older messages load to find the one gone to; the oldest loaded so
    /// far is from `back_to`.
    Seeking { back_to: Option<SystemTime> },
    /// Files of messages being sent upload: the first one's name, how many,
    /// and how far, from 0 to 1.
    Uploading {
        name: String,
        files: usize,
        done: f32,
    },
}

impl Status {
    fn line(&self) -> String {
        match self {
            Self::Problem(problem) => problem.clone(),
            Self::Seeking { back_to: None } => "Looking for the message…".to_owned(),
            Self::Seeking {
                back_to: Some(time),
            } => format!("Looking for the message… back to {}", day(*time)),
            Self::Uploading { name, files, done } => {
                let what = match files {
                    1 => name.clone(),
                    n => format!("{n} files"),
                };
                format!("Uploading {what} · {:.0}%", done * 100.0)
            }
        }
    }

    /// The button closing it: dismiss a problem, stop looking.
    fn close(&self) -> Option<(Message, &'static str)> {
        match self {
            Self::Problem(_) => Some((Message::DismissError, "Dismiss")),
            Self::Seeking { .. } => Some((Message::StopSeeking, "Stop looking")),
            Self::Uploading { .. } => None,
        }
    }
}

/// `time`'s day: "3 March", with the year if not this one.
fn day(time: SystemTime) -> String {
    let time: DateTime<Local> = time.into();
    if time.year() == Local::now().year() {
        time.format("%-d %B").to_string()
    } else {
        time.format("%-d %B %Y").to_string()
    }
}

/// The pill of `room`.
pub fn view<'a>(room: &OwnedRoomId, status: &Status) -> Element<'a, app::Message> {
    let mut line = row![text(status.line()).size(12)]
        .spacing(8)
        .align_y(Alignment::Center);
    if let Some((message, label)) = status.close() {
        line = line.push(hint(
            button(text("×").size(14))
                .padding([0, 4])
                .style(style::text_button)
                .on_press(message.to(room)),
            label,
            Position::Top,
        ));
    }
    container(line)
        .padding([4, 12])
        .style(style::status_pill(matches!(status, Status::Problem(_))))
        .into()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn says_what_is_waited_on() {
        let uploading = |files| Status::Uploading {
            name: "report.pdf".into(),
            files,
            done: 0.452,
        };
        assert_eq!(uploading(1).line(), "Uploading report.pdf · 45%");
        assert_eq!(uploading(3).line(), "Uploading 3 files · 45%");
        let long_ago = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400 * 400);
        assert!(
            Status::Seeking {
                back_to: Some(long_ago)
            }
            .line()
            .ends_with("1971"),
            "a past year is named"
        );
        assert!(Status::Seeking { back_to: None }.close().is_some());
        assert!(uploading(1).close().is_none());
    }
}
