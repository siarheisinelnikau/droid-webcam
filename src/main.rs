use droid_camera_source::{FPS, WIDTH};
use droid_webcam::host;
use droid_webcam::protocol::{Invite, Pairing, Result, Sender};
use droid_webcam::video::{Feed, State, synthetic_jpeg};
use std::io::{self, Write};
use std::net::{SocketAddr, SocketAddrV4, TcpListener};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

fn main() {
    if let Err(error) = run() {
        eprintln!("Droid Webcam: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["serve", address] => serve(address.parse()?, false),
        ["serve", address, "--camera"] => serve(address.parse()?, true),
        ["send-test"] => send_test(150),
        ["send-test", count] => send_test(count.parse()?),
        [] | ["--help" | "-h"] => {
            println!("Droid Webcam\n  serve <IPv4:port> [--camera]\n  send-test [frame-count]");
            Ok(())
        }
        _ => Err("unknown command; see --help".into()),
    }
}

fn serve(address: SocketAddrV4, enable_camera: bool) -> Result<()> {
    if address.ip().is_unspecified() || address.ip().is_multicast() || address.ip().is_broadcast() {
        return Err("bind a specific unicast IPv4 address".into());
    }
    if enable_camera {
        return Err("the virtual camera requires Windows 11".into());
    }
    let listener = TcpListener::bind(address)?;
    let SocketAddr::V4(address) = listener.local_addr()? else {
        unreachable!()
    };
    let pairing = Pairing::new(address)?;

    println!("Paste this one-time invitation into Droid Webcam and tap Start within 3 minutes.");
    println!("{}", *pairing.invite().code());
    println!("Press Enter here to stop.");
    io::stdout().flush()?;

    let stop = stop_on_enter();
    let feed = Feed::default();
    thread::scope(|scope| {
        let worker = scope.spawn(|| host::receive(listener, pairing, stop.clone(), &feed, report));
        worker.join().map_err(|_| "receiver thread failed")?
    })
}

fn stop_on_enter() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    thread::spawn(move || {
        let mut line = String::new();
        if io::stdin()
            .read_line(&mut line)
            .is_ok_and(|bytes| bytes != 0)
        {
            signal.store(true, Ordering::Relaxed);
        }
    });
    stop
}

fn report(state: State, details: Option<&str>) {
    let message = match state {
        State::Waiting => "Waiting for the phone to confirm pairing",
        State::Paired => "Phone paired. Waiting for video",
        State::Streaming => "Streaming",
        State::Stopped => "Stopped",
        State::Disconnected => "Disconnected",
        State::Expired => "Invitation expired. Start a new session to pair again",
    };
    match details {
        Some(details) => println!("{message} ({details})."),
        None => println!("{message}."),
    }
}

fn send_test(count: usize) -> Result<()> {
    println!("Paste the invitation, then press Enter.");
    let mut code = Zeroizing::new(String::new());
    io::stdin().read_line(&mut code)?;
    let invite = Invite::parse(&code)?;
    let mut sender = Sender::connect(&invite)?;
    let period = Duration::from_nanos(1_000_000_000 / FPS);
    for index in 0..count {
        let start = Instant::now();
        sender.send(&synthetic_jpeg((index % WIDTH) * 8 % WIDTH)?)?;
        thread::sleep(period.saturating_sub(start.elapsed()));
    }
    sender.stop()
}
