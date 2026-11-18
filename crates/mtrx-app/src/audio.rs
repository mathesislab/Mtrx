//! Audio messages play in the timeline: a button that plays and pauses, a
//! scrubber (the sender's waveform for voice messages) and the time. One
//! plays at a time, on the audio thread ([`output`]); where the others
//! stopped is kept while the room is open. Core gives the file to play.

mod output;

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use iced::{
    Alignment, Background, Border, Element, Length, Point, Rectangle, Renderer, Subscription, Task,
    Theme,
    futures::{SinkExt, channel::mpsc::Sender},
    mouse,
    task::Handle,
    widget::{
        button,
        canvas::{self, Frame, Geometry, Path},
        row, slider, text,
    },
};
use mtrx_core::{Media, MediaRef, Session, human_size, playback_time};

use crate::{
    app,
    downloads::File,
    hint::{Position, hint},
    icons, media, style,
};
use output::{Command, Event};

/// The play button's size.
const BUTTON: f32 = 32.0;
/// The height of a voice message's waveform.
const WAVEFORM_HEIGHT: f32 = 28.0;
/// The time's box, so the scrubber keeps its width as the digits change;
/// wider for an hour or more.
const TIME_WIDTH: f32 = 40.0;
const LONG_TIME_WIDTH: f32 = 56.0;

/// The message playing and where others stopped, while the room is open.
#[derive(Default)]
pub struct Audio {
    /// The message playing, paused, fetching or failed.
    now: Option<Now>,
    /// Where others stopped, while the room is open.
    positions: HashMap<MediaRef, Duration>,
    /// The audio thread's, once started.
    output: Option<output::Commands>,
    /// Aborted by another play or [`Message::Stop`].
    fetch: Option<Handle>,
    /// The [`Command::Play`]s sent: events about earlier ones are late.
    plays: u64,
}

struct Now {
    file: File,
    state: State,
    position: Duration,
    duration: Option<Duration>,
    /// Where the scrubber is being dragged.
    drag: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    /// From 0 to 1, if the length is known.
    Fetching(Option<f32>),
    Playing,
    Paused,
    /// Why, in English.
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    /// The button: play, pause, cancel or retry.
    Toggle(File),
    /// The scrubber moves.
    Drag(MediaRef, Duration),
    /// The scrubber is released, or the waveform clicked.
    Seek(MediaRef, Duration),
    Progress(MediaRef, Option<f32>),
    Fetched(MediaRef, Result<PathBuf, String>),
    /// From the audio thread, about its `Play` of this number.
    Output(u64, Event),
    /// Another room opened.
    Stop,
}

/// A step of fetching a message's audio.
enum Step {
    Progress(Option<f32>),
    Done(Result<PathBuf, String>),
}

impl Audio {
    pub fn update(&mut self, session: &Arc<Session>, message: Message) -> Task<Message> {
        self.handle(message, |media| fetch(session.clone(), media))
    }

    /// [`Audio::update`], with `fetch` to get a message's file.
    fn handle(
        &mut self,
        message: Message,
        fetch: impl FnOnce(&Media) -> Task<Message>,
    ) -> Task<Message> {
        match message {
            Message::Toggle(file) => {
                let state = self.now_of(&file.media.source).map(|now| now.state.clone());
                match state {
                    Some(State::Playing) => {
                        self.send(Command::Pause);
                        self.set_state(State::Paused);
                    }
                    Some(State::Paused) => {
                        self.send(Command::Resume);
                        self.set_state(State::Playing);
                    }
                    // A click cancels.
                    Some(State::Fetching(_)) => self.leave(),
                    Some(State::Failed(_)) | None => return self.start(file, fetch),
                }
            }
            Message::Drag(source, at) => match self.now_of_mut(&source) {
                Some(now) => now.drag = Some(at),
                None => {
                    self.positions.insert(source, at);
                }
            },
            Message::Seek(source, at) => match self.now_of_mut(&source) {
                Some(now) => {
                    let at = now.drag.take().unwrap_or(at);
                    now.position = at;
                    if matches!(now.state, State::Playing | State::Paused) {
                        self.send(Command::Seek(at));
                    }
                }
                // Where it will start.
                None => {
                    self.positions.insert(source, at);
                }
            },
            Message::Progress(source, fraction) => {
                if let Some(now) = self.now_of_mut(&source)
                    && let State::Fetching(_) = now.state
                {
                    now.state = State::Fetching(fraction);
                }
            }
            Message::Fetched(source, result) => {
                let fetching =
                    (self.now_of(&source)).filter(|now| matches!(now.state, State::Fetching(_)));
                let Some(from) = fetching.map(|now| now.position) else {
                    // Cancelled meanwhile.
                    return Task::none();
                };
                let state = match (result, &self.output) {
                    (Ok(path), Some(output)) => {
                        output.send(Command::Play { path, from });
                        self.plays += 1;
                        State::Playing
                    }
                    (Ok(_), None) => State::Failed(output::NO_OUTPUT.into()),
                    (Err(reason), _) => State::Failed(reason),
                };
                self.set_state(state);
                self.fetch = None;
            }
            Message::Output(_, Event::Ready(commands)) => {
                // A new thread plays nothing: what played before stopped.
                self.output = Some(commands);
                self.plays = 0;
                if self.now.as_ref().is_some_and(Now::started) {
                    self.leave();
                }
            }
            Message::Output(play, event) if play == self.plays => self.output_event(event),
            // About a file played before.
            Message::Output(..) => {}
            Message::Stop => {
                self.leave();
                self.positions.clear();
            }
        }
        Task::none()
    }

    fn output_event(&mut self, event: Event) {
        if !self.now.as_ref().is_some_and(Now::started) {
            return;
        }
        if let Event::Ended = event {
            // Back to the start.
            self.now = None;
            return;
        }
        let Some(now) = &mut self.now else {
            return;
        };
        match event {
            Event::Started { duration } => now.duration = duration.or(now.duration),
            Event::At(at) => now.position = at,
            Event::Failed(reason) => now.state = State::Failed(reason),
            Event::Ended | Event::Ready(_) => {}
        }
    }

    /// Fetch `file`'s audio to play it, from where it stopped; the message
    /// playing before stops, keeping its position.
    fn start(&mut self, file: File, fetch: impl FnOnce(&Media) -> Task<Message>) -> Task<Message> {
        self.leave();
        let position = (self.positions.remove(&file.media.source)).unwrap_or_default();
        let (task, handle) = fetch(&file.media).abortable();
        self.fetch = Some(handle.abort_on_drop());
        self.now = Some(Now {
            duration: file.media.duration,
            file,
            state: State::Fetching(None),
            position,
            drag: None,
        });
        task
    }

    /// The message playing (or paused, fetching, failed) stops; where it
    /// was is kept.
    fn leave(&mut self) {
        self.fetch = None;
        let Some(now) = self.now.take() else {
            return;
        };
        if now.started() {
            self.send(Command::Stop);
        }
        if !now.position.is_zero() {
            self.positions.insert(now.file.media.source, now.position);
        }
    }

    fn send(&self, command: Command) {
        if let Some(output) = &self.output {
            output.send(command);
        }
    }

    fn set_state(&mut self, state: State) {
        if let Some(now) = &mut self.now {
            now.state = state;
        }
    }

    fn now_of(&self, source: &MediaRef) -> Option<&Now> {
        self.now
            .as_ref()
            .filter(|now| now.file.media.source == *source)
    }

    fn now_of_mut(&mut self, source: &MediaRef) -> Option<&mut Now> {
        self.now
            .as_mut()
            .filter(|now| now.file.media.source == *source)
    }

    /// The audio thread's events.
    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::run(output::events).map(|(play, event)| Message::Output(play, event))
    }

    /// A message's audio: the button, the scrubber and the time (or why it
    /// failed).
    pub fn view<'a>(&self, file: &File) -> Element<'a, app::Message> {
        let media = &file.media;
        let now = self.now_of(&media.source);
        let state = now.map(|now| &now.state);
        let position = match now {
            Some(now) => {
                (now.drag).or((now.started() || !now.position.is_zero()).then_some(now.position))
            }
            None => self.positions.get(&media.source).copied(),
        };
        let duration = now.and_then(|now| now.duration).or(media.duration);
        let scrubber = if media.voice && !media.waveform.is_empty() {
            waveform(media, position, duration)
        } else {
            scrubber(media, position, duration)
        };
        let time: Element<'a, app::Message> = match state {
            Some(State::Failed(reason)) => text(reason.clone())
                .size(12)
                .style(style::problem_text)
                .into(),
            _ => {
                let long = duration.is_some_and(|d| d >= Duration::from_secs(3600));
                text(time_label(position, duration, media.size))
                    .size(12)
                    .style(text::secondary)
                    .width(if long { LONG_TIME_WIDTH } else { TIME_WIDTH })
                    .align_x(Alignment::End)
                    .into()
            }
        };
        row![play_button(file, state), scrubber, time]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
    }
}

impl Now {
    /// Sent to the audio thread: playing or paused.
    fn started(&self) -> bool {
        matches!(self.state, State::Playing | State::Paused)
    }
}

fn to(message: Message) -> app::Message {
    app::Message::Media(media::Message::Audio(message))
}

/// Get a message's file to play, with progress.
fn fetch(session: Arc<Session>, media: &Media) -> Task<Message> {
    let (media, source) = (media.clone(), media.source.clone());
    let steps = iced::stream::channel(16, async move |mut steps: Sender<Step>| {
        let mut progress = steps.clone();
        let result = session
            .audio_file(&media, move |fraction| {
                let _ = progress.try_send(Step::Progress(fraction));
            })
            .await;
        let _ = steps
            .send(Step::Done(result.map_err(|e| e.to_string())))
            .await;
    });
    Task::run(steps, move |step| match step {
        Step::Progress(fraction) => Message::Progress(source.clone(), fraction),
        Step::Done(result) => Message::Fetched(source.clone(), result),
    })
}

/// The time shown: where it is once started, else its length, else its
/// size.
fn time_label(position: Option<Duration>, duration: Option<Duration>, size: Option<u64>) -> String {
    match (position, duration, size) {
        (Some(position), ..) => playback_time(position),
        (None, Some(duration), _) => playback_time(duration),
        (None, None, Some(size)) => human_size(size),
        (None, None, None) => String::new(),
    }
}

/// Plays and pauses; while fetching, a ring with how far (a click
/// cancels); after a failure, tries again.
fn play_button<'a>(file: &File, state: Option<&State>) -> Element<'a, app::Message> {
    let white = |theme: &Theme| theme.extended_palette().primary.base.text;
    let (icon, tip): (Element<'a, app::Message>, _) = match state {
        Some(State::Fetching(fraction)) => (icons::ring(BUTTON, *fraction), "Cancel"),
        Some(State::Playing) => (icons::pause(BUTTON * 0.6, white), "Pause"),
        Some(State::Failed(_)) => (icons::retry(BUTTON * 0.6, white), "Try again"),
        Some(State::Paused) | None => (icons::play(BUTTON * 0.6, white), "Play"),
    };
    let look: fn(&Theme, button::Status) -> button::Style = match state {
        Some(State::Fetching(_)) => style::text_button,
        _ => round,
    };
    let button = button(
        iced::widget::container(icon)
            .center_x(BUTTON)
            .center_y(BUTTON),
    )
    .padding(0)
    .style(look)
    .on_press(to(Message::Toggle(file.clone())));
    hint(button, tip, Position::Top)
}

/// The play button: round, in the accent colour.
fn round(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => palette.primary.strong.color,
        _ => palette.primary.base.color,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: palette.primary.base.text,
        border: Border::default().rounded(BUTTON / 2.0),
        ..Default::default()
    }
}

/// A slider: dragging moves the time shown, releasing seeks.
fn scrubber<'a>(
    media: &Media,
    position: Option<Duration>,
    duration: Option<Duration>,
) -> Element<'a, app::Message> {
    let end = duration.unwrap_or_default().as_secs_f32();
    let at = position.unwrap_or_default();
    let (moved, released) = (media.source.clone(), media.source.clone());
    slider(0.0..=end, at.as_secs_f32().min(end), move |seconds| {
        to(Message::Drag(
            moved.clone(),
            Duration::from_secs_f32(seconds),
        ))
    })
    .step(0.1_f32)
    .on_release(to(Message::Seek(released, at)))
    .width(Length::Fill)
    .into()
}

/// The sender's waveform, the bars played in the accent colour: a click
/// or a drag seeks.
fn waveform<'a>(
    media: &Media,
    position: Option<Duration>,
    duration: Option<Duration>,
) -> Element<'a, app::Message> {
    let played = match (position, duration) {
        (Some(position), Some(duration)) if !duration.is_zero() => {
            (position.as_secs_f32() / duration.as_secs_f32()).min(1.0)
        }
        _ => 0.0,
    };
    canvas::Canvas::new(Waveform {
        bars: media.waveform.clone(),
        played,
        duration: duration.unwrap_or_default(),
        source: media.source.clone(),
    })
    .width(Length::Fill)
    .height(WAVEFORM_HEIGHT)
    .into()
}

struct Waveform {
    bars: Vec<f32>,
    /// From 0 to 1.
    played: f32,
    duration: Duration,
    source: MediaRef,
}

impl Waveform {
    /// The time at `x` across `bounds`.
    fn at(&self, x: f32, bounds: Rectangle) -> Duration {
        let fraction = ((x - bounds.x) / bounds.width.max(1.0)).clamp(0.0, 1.0);
        self.duration.mul_f32(fraction)
    }
}

impl canvas::Program<app::Message> for Waveform {
    /// Pressed on it, until released.
    type State = bool;

    fn update(
        &self,
        pressed: &mut bool,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<app::Message>> {
        let canvas::Event::Mouse(event) = event else {
            return None;
        };
        let x = cursor.position()?.x;
        let message = match event {
            mouse::Event::ButtonPressed(mouse::Button::Left) if cursor.is_over(bounds) => {
                *pressed = true;
                Message::Drag(self.source.clone(), self.at(x, bounds))
            }
            mouse::Event::CursorMoved { .. } if *pressed => {
                Message::Drag(self.source.clone(), self.at(x, bounds))
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) if *pressed => {
                *pressed = false;
                Message::Seek(self.source.clone(), self.at(x, bounds))
            }
            _ => return None,
        };
        Some(canvas::Action::publish(to(message)).and_capture())
    }

    fn draw(
        &self,
        _pressed: &bool,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let palette = theme.extended_palette();
        let count = self.bars.len().max(1) as f32;
        let step = bounds.width / count;
        let width = (step * 0.6).max(1.0);
        for (i, bar) in self.bars.iter().enumerate() {
            let height = (bar * bounds.height).max(2.0);
            let x = i as f32 * step + (step - width) / 2.0;
            let top = Point::new(x, (bounds.height - height) / 2.0);
            let color = if (i as f32 + 0.5) / count <= self.played {
                palette.primary.base.color
            } else {
                palette.background.strong.color
            };
            let path =
                Path::rounded_rectangle(top, iced::Size::new(width, height), (width / 2.0).into());
            frame.fill(&path, color);
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _pressed: &bool,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::Receiver;

    use iced_test::simulator::{Simulator, click};
    use mtrx_core::{MediaKind, OwnedRoomId};

    use super::*;
    use crate::fonts;

    fn file(name: &str, voice: bool) -> File {
        let room: OwnedRoomId = "!room:example.org".try_into().unwrap();
        File {
            media: Media {
                kind: MediaKind::Audio,
                filename: format!("{name}.ogg"),
                size: Some(18_045),
                mimetype: Some("audio/ogg".into()),
                width: None,
                height: None,
                source: MediaRef::plain(&format!("mxc://example.org/{name}")),
                duration: Some(Duration::from_millis(42_600)),
                voice,
                waveform: if voice {
                    (0..40).map(|i| i as f32 / 39.0).collect()
                } else {
                    Vec::new()
                },
            },
            room,
            event: None,
        }
    }

    /// Audio with a fake output, and what it was told.
    fn audio() -> (Audio, Receiver<Command>) {
        let (commands, sent) = output::Commands::fake();
        let mut audio = Audio::default();
        let _ = audio.handle(Message::Output(0, Event::Ready(commands)), no_fetch);
        (audio, sent)
    }

    fn no_fetch(_: &Media) -> Task<Message> {
        Task::none()
    }

    fn sent(commands: &Receiver<Command>) -> Vec<Command> {
        commands.try_iter().collect()
    }

    /// Play `file`: its fetch done, the thread started.
    fn play(audio: &mut Audio, file: &File) {
        let source = file.media.source.clone();
        let _ = audio.handle(Message::Toggle(file.clone()), no_fetch);
        let path = PathBuf::from(format!("/cache/{}", file.media.filename));
        let _ = audio.handle(Message::Fetched(source, Ok(path)), no_fetch);
        let started = Event::Started {
            duration: Some(Duration::from_secs(42)),
        };
        let _ = audio.handle(Message::Output(audio.plays, started), no_fetch);
    }

    fn at(audio: &mut Audio, seconds: u64) {
        let event = Event::At(Duration::from_secs(seconds));
        let _ = audio.handle(Message::Output(audio.plays, event), no_fetch);
    }

    #[test]
    fn playing_another_stops_the_first_where_it_was() {
        let (mut audio, commands) = audio();
        let (a, b) = (file("a", true), file("b", false));
        play(&mut audio, &a);
        assert_eq!(
            sent(&commands),
            [Command::Play {
                path: "/cache/a.ogg".into(),
                from: Duration::ZERO
            }]
        );
        at(&mut audio, 12);
        play(&mut audio, &b);
        assert_eq!(
            sent(&commands),
            [
                Command::Stop,
                Command::Play {
                    path: "/cache/b.ogg".into(),
                    from: Duration::ZERO
                }
            ]
        );
        assert_eq!(audio.now_of(&b.media.source).unwrap().state, State::Playing);
        assert_eq!(
            audio.positions.get(&a.media.source),
            Some(&Duration::from_secs(12))
        );

        // Late news of the first is not the second's.
        let late = Event::At(Duration::from_secs(13));
        let _ = audio.handle(Message::Output(audio.plays - 1, late), no_fetch);
        assert_eq!(audio.now.as_ref().unwrap().position, Duration::ZERO);

        // Back to the first: from where it stopped.
        play(&mut audio, &a);
        assert_eq!(
            sent(&commands),
            [
                Command::Stop,
                Command::Play {
                    path: "/cache/a.ogg".into(),
                    from: Duration::from_secs(12)
                }
            ]
        );
    }

    #[test]
    fn the_end_goes_back_to_the_start() {
        let (mut audio, _commands) = audio();
        let a = file("a", true);
        play(&mut audio, &a);
        at(&mut audio, 40);
        let _ = audio.handle(Message::Output(audio.plays, Event::Ended), no_fetch);
        assert!(audio.now.is_none());
        assert!(audio.positions.is_empty());
    }

    #[test]
    fn stop_forgets_where_they_were_and_the_fetch() {
        let (mut audio, commands) = audio();
        let (a, b) = (file("a", true), file("b", false));
        play(&mut audio, &a);
        at(&mut audio, 5);
        let _ = audio.handle(Message::Toggle(b.clone()), |_| {
            Task::future(std::future::pending::<Message>())
        });
        assert!(audio.fetch.is_some());
        let _ = sent(&commands);
        let _ = audio.handle(Message::Stop, no_fetch);
        assert!(audio.now.is_none());
        assert!(audio.fetch.is_none());
        assert!(audio.positions.is_empty());
        // The fetch was all that ran.
        assert!(sent(&commands).is_empty());
    }

    #[test]
    fn seeking_one_that_isnt_playing_sets_where_it_starts() {
        let (mut audio, commands) = audio();
        let a = file("a", false);
        let _ = audio.handle(
            Message::Seek(a.media.source.clone(), Duration::from_secs(20)),
            no_fetch,
        );
        assert!(audio.now.is_none());
        play(&mut audio, &a);
        assert_eq!(
            sent(&commands),
            [Command::Play {
                path: "/cache/a.ogg".into(),
                from: Duration::from_secs(20)
            }]
        );
        // While it plays, a seek goes to the thread.
        let _ = audio.handle(
            Message::Seek(a.media.source.clone(), Duration::from_secs(30)),
            no_fetch,
        );
        assert_eq!(sent(&commands), [Command::Seek(Duration::from_secs(30))]);
    }

    #[test]
    fn a_click_pauses_resumes_and_cancels() {
        let (mut audio, commands) = audio();
        let a = file("a", true);
        play(&mut audio, &a);
        let _ = sent(&commands);
        let _ = audio.handle(Message::Toggle(a.clone()), no_fetch);
        let _ = audio.handle(Message::Toggle(a.clone()), no_fetch);
        assert_eq!(sent(&commands), [Command::Pause, Command::Resume]);

        let b = file("b", true);
        let _ = audio.handle(Message::Toggle(b.clone()), no_fetch);
        assert!(matches!(
            audio.now_of(&b.media.source).unwrap().state,
            State::Fetching(None)
        ));
        let _ = audio.handle(Message::Toggle(b.clone()), no_fetch);
        assert!(audio.now.is_none());
        // Its file came anyway: nothing plays.
        let _ = audio.handle(
            Message::Fetched(b.media.source.clone(), Ok("/cache/b.ogg".into())),
            no_fetch,
        );
        assert_eq!(sent(&commands), [Command::Stop]);
    }

    #[test]
    fn a_voice_message_shows_its_length_and_plays_on_a_click() {
        let (audio, _commands) = audio();
        let voice = file("voice", true);
        let mut ui = Simulator::with_size(
            fonts::settings(),
            iced::Size::new(400.0, 60.0),
            audio.view(&voice),
        );
        assert!(ui.find("0:42").is_ok());
        ui.point_at((BUTTON / 2.0, BUTTON / 2.0));
        let _ = ui.simulate(click());
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            matches!(
                &messages[..],
                [app::Message::Media(media::Message::Audio(Message::Toggle(f)))] if *f == voice
            ),
            "{messages:?}"
        );
    }
}
