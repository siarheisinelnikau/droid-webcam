use crate::protocol::{Pairing, Result};
use crate::video::{Feed, State, decode_nv12};
use std::io;
use std::net::TcpListener;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

pub fn receive(
    listener: TcpListener,
    mut pairing: Pairing,
    stop: Arc<AtomicBool>,
    feed: &Feed,
    mut report: impl FnMut(State, Option<&str>),
) -> Result<()> {
    listener.set_nonblocking(true)?;
    report(State::Waiting, None);
    let mut receiver = loop {
        let state = if stop.load(Ordering::Relaxed) {
            Some(State::Stopped)
        } else if pairing.expired() {
            Some(State::Expired)
        } else {
            None
        };
        if let Some(state) = state {
            feed.set_state(state);
            report(state, None);
            return Ok(());
        }
        match listener.accept() {
            Ok((socket, _)) => match pairing.accept(socket, stop.clone()) {
                Ok(receiver) => break receiver,
                Err(error) if pairing.used() => {
                    feed.set_state(State::Disconnected);
                    report(State::Disconnected, Some(&error.to_string()));
                    return Ok(());
                }
                Err(_) => {} // An unauthenticated peer cannot change the feed state.
            },
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error.into()),
        }
    };
    drop(listener);
    drop(pairing);
    let settings = receiver.settings();
    let mode = settings.mode();
    feed.configure(mode);
    report(
        State::Paired,
        Some(&format!(
            "{mode}, quality {}, {} camera",
            settings.quality(),
            if settings.front() { "front" } else { "rear" }
        )),
    );

    let result = (|| -> Result<()> {
        let mut frame = Vec::new();
        let mut first = true;
        while receiver.next_frame(&mut frame)? {
            let pixels = decode_nv12(&frame, mode)?;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            feed.publish(mode, pixels)?;
            receiver.acknowledge_frame()?;
            if first {
                report(State::Streaming, None);
                first = false;
            }
        }
        Ok(())
    })();
    if stop.load(Ordering::Relaxed) || result.is_ok() {
        feed.set_state(State::Stopped);
        report(State::Stopped, None);
    } else if let Err(error) = result {
        feed.set_state(State::Disconnected);
        report(State::Disconnected, Some(&error.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Invite, Sender};
    use crate::video::synthetic_jpeg;
    use std::net::SocketAddr;
    use std::sync::mpsc;

    fn loopback(test: impl FnOnce(Sender, &Feed, &AtomicBool)) -> Vec<State> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let SocketAddr::V4(address) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        let pairing = Pairing::new(address).unwrap();
        let invite = Invite::parse(&pairing.invite().code()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let feed = Feed::default();
        let (tx, rx) = mpsc::channel();
        thread::scope(|scope| {
            scope.spawn(|| {
                receive(listener, pairing, stop.clone(), &feed, move |state, _| {
                    tx.send(state).unwrap();
                })
                .unwrap()
            });
            let sender = Sender::connect(&invite).unwrap();
            test(sender, &feed, &stop);
        });
        assert!(feed.latest().is_none());
        rx.iter().collect()
    }

    #[test]
    fn loopback_stop_discards_frame() {
        let states = loopback(|mut sender, feed, _| {
            sender
                .send(&synthetic_jpeg(0, droid_camera_source::DEFAULT_MODE, 80).unwrap())
                .unwrap();
            assert_eq!(feed.state(), State::Streaming);
            assert!(feed.latest().is_some());
            sender.stop().unwrap();
        });
        assert_eq!(
            states,
            [
                State::Waiting,
                State::Paired,
                State::Streaming,
                State::Stopped
            ]
        );
    }

    #[test]
    fn loopback_disconnect_and_malformed_jpeg() {
        assert_eq!(
            loopback(|mut sender, _, _| {
                sender
                    .send(&synthetic_jpeg(0, droid_camera_source::DEFAULT_MODE, 80).unwrap())
                    .unwrap();
                drop(sender);
            }),
            [
                State::Waiting,
                State::Paired,
                State::Streaming,
                State::Disconnected
            ]
        );
        assert_eq!(
            loopback(|mut sender, _, _| {
                assert!(sender.send(b"not a JPEG").is_err());
            }),
            [State::Waiting, State::Paired, State::Disconnected]
        );
    }

    #[test]
    fn host_stop_cancels_an_idle_paired_sender() {
        let states = loopback(|sender, _, stop| {
            stop.store(true, Ordering::Relaxed);
            // Keep the connection alive long enough for cancellation to interrupt the read.
            thread::sleep(Duration::from_millis(250));
            drop(sender);
        });
        assert_eq!(states, [State::Waiting, State::Paired, State::Stopped]);
    }
}
