//! The audio thread: plays one file at a time with rodio, as the app
//! commands, and says where it is. It holds the output device only while
//! a file plays or is paused; idle, it waits for a command.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    time::Duration,
};

use iced::futures::{
    Stream,
    channel::mpsc::{UnboundedSender, unbounded},
};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

/// How often a playing file says where it is.
const TICK: Duration = Duration::from_millis(100);
pub const NO_OUTPUT: &str = "No sound output";
const CANT_PLAY: &str = "Can't play this file";

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Play the file at `path` from `from`, instead of the one before.
    Play {
        path: PathBuf,
        from: Duration,
    },
    Pause,
    Resume,
    Seek(Duration),
    /// Stop, letting go of the output.
    Stop,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// The thread runs: where to send it commands.
    Ready(Commands),
    /// The file plays: its length, if it says.
    Started { duration: Option<Duration> },
    /// Where the file is, every [`TICK`] while it plays.
    At(Duration),
    /// The file played to its end.
    Ended,
    /// Why it can't play.
    Failed(String),
}

/// Where the app sends the thread its commands.
#[derive(Debug, Clone)]
pub struct Commands(Sender<Command>);

impl Commands {
    pub fn send(&self, command: Command) {
        // The thread ends only with the app's window.
        let _ = self.0.send(command);
    }

    /// Commands that go to the receiver instead (tests).
    #[cfg(test)]
    pub fn fake() -> (Self, Receiver<Command>) {
        let (sender, receiver) = mpsc::channel();
        (Self(sender), receiver)
    }
}

/// The thread's events, each with the number of the [`Command::Play`] it
/// is about (0 before the first): it starts the thread.
pub fn events() -> impl Stream<Item = (u64, Event)> {
    let (sender, receiver) = unbounded();
    let (commands, inbox) = mpsc::channel();
    let _ = sender.unbounded_send((0, Event::Ready(Commands(commands))));
    let spawned = std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || run(&inbox, &sender));
    if let Err(e) = spawned {
        tracing::warn!("could not start the audio thread: {e}");
    }
    receiver
}

/// A file playing or paused, and the output it plays on.
struct Playing {
    player: Player,
    _output: MixerDeviceSink,
}

/// Carry out commands until the app is gone.
fn run(inbox: &Receiver<Command>, events: &UnboundedSender<(u64, Event)>) {
    let (mut playing, mut play) = (None::<Playing>, 0);
    loop {
        let ticking = (playing.as_ref()).is_some_and(|p| !p.player.is_paused());
        let command = if ticking {
            match inbox.recv_timeout(TICK) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match inbox.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        let event = match command {
            None => tick(&mut playing),
            Some(Command::Play { path, from }) => {
                play += 1;
                // The one before lets go of the output first.
                playing = None;
                match start(&path, from) {
                    Ok((started, duration)) => {
                        playing = Some(started);
                        Some(Event::Started { duration })
                    }
                    Err(reason) => Some(Event::Failed(reason.into())),
                }
            }
            Some(Command::Stop) => {
                playing = None;
                None
            }
            Some(command) => (playing.as_ref()).and_then(|p| control(&p.player, command)),
        };
        if let Some(event) = event
            && events.unbounded_send((play, event)).is_err()
        {
            return;
        }
    }
}

/// Open the default output and the file, and play it from `from`; its
/// length, if it says.
fn start(path: &Path, from: Duration) -> Result<(Playing, Option<Duration>), &'static str> {
    let mut output = DeviceSinkBuilder::open_default_sink().map_err(|_| NO_OUTPUT)?;
    output.log_on_drop(false);
    let file = std::fs::File::open(path).map_err(|_| CANT_PLAY)?;
    let mut decoder = Decoder::try_from(file).map_err(|_| CANT_PLAY)?;
    let duration = decoder.total_duration();
    if !from.is_zero() {
        // Before it plays, so nothing of the start sounds.
        let _ = decoder.try_seek(from);
    }
    let player = Player::connect_new(output.mixer());
    player.append(decoder);
    let playing = Playing {
        player,
        _output: output,
    };
    Ok((playing, duration))
}

/// Where the file is, or its end: then the output goes.
fn tick(playing: &mut Option<Playing>) -> Option<Event> {
    if playing.as_ref()?.player.empty() {
        *playing = None;
        return Some(Event::Ended);
    }
    (playing.as_ref()).map(|p| Event::At(p.player.get_pos()))
}

/// Pause, resume or seek.
fn control(player: &Player, command: Command) -> Option<Event> {
    match command {
        Command::Pause => {
            player.pause();
            Some(Event::At(player.get_pos()))
        }
        Command::Resume => {
            player.play();
            None
        }
        Command::Seek(at) => {
            let _ = player.try_seek(at);
            Some(Event::At(player.get_pos()))
        }
        Command::Play { .. } | Command::Stop => None,
    }
}
