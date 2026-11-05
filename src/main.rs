use droid_camera_source::{DEFAULT_MODE, VideoMode};
use droid_webcam::{
    host,
    protocol::{CaptureSettings, Invite, Pairing, Result, Sender},
    video::{Feed, State, synthetic_jpeg},
};
use std::{
    io::{self, Write},
    net::{SocketAddr, SocketAddrV4, TcpListener},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[cfg(target_os = "linux")]
mod linux_host;
#[cfg(windows)]
mod windows_host;
#[cfg(target_os = "linux")]
use linux_host::CameraPublisher;

fn main() {
    if let Err(error) = run() {
        eprintln!("Droid Webcam: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["serve", address, options @ ..] => {
            let mut camera = false;
            #[cfg(target_os = "linux")]
            let mut device = PathBuf::from("/dev/video10");
            #[cfg(not(target_os = "linux"))]
            let device = PathBuf::from("/dev/video10");
            let mut remaining = options;
            while !remaining.is_empty() {
                match remaining {
                    ["--camera", tail @ ..] => {
                        camera = true;
                        remaining = tail;
                    }
                    #[cfg(target_os = "linux")]
                    ["--device", path, tail @ ..] => {
                        device = PathBuf::from(path);
                        remaining = tail;
                    }
                    _ => return Err("unknown serve option; see --help".into()),
                }
            }
            serve(address.parse()?, camera, &device)
        }
        ["send-test", options @ ..] => {
            let options = TestOptions::parse(options)?;
            send_test(options)
        }
        ["test-pattern", options @ ..] => test_pattern(TestOptions::parse(options)?),
        #[cfg(windows)]
        ["install-source"] => windows_host::install(),
        #[cfg(windows)]
        ["uninstall-source"] => windows_host::uninstall(),
        [] | ["--help" | "-h"] => {
            let device_option = if cfg!(target_os = "linux") {
                " [--device /dev/video10]"
            } else {
                ""
            };
            println!(
                "Droid Webcam\n  serve <IPv4:port> [--camera]{device_option}\n  send-test [frame-count] [--mode 1280x720@30] [--quality 80]\n  test-pattern [--mode 1280x720@30]{device_option}"
            );
            #[cfg(windows)]
            println!("  install-source\n  uninstall-source");
            Ok(())
        }
        _ => Err("unknown command; see --help".into()),
    }
}

struct TestOptions {
    mode: VideoMode,
    quality: u8,
    count: usize,
    device: PathBuf,
}

impl TestOptions {
    fn parse(mut arguments: &[&str]) -> Result<Self> {
        let mut options = Self {
            mode: DEFAULT_MODE,
            quality: 80,
            count: 150,
            device: "/dev/video10".into(),
        };
        if let Some(first) = arguments
            .first()
            .filter(|argument| !argument.starts_with('-'))
        {
            options.count = first.parse()?;
            arguments = &arguments[1..];
        }
        while !arguments.is_empty() {
            match arguments {
                ["--mode", mode, tail @ ..] => {
                    options.mode = mode.parse()?;
                    arguments = tail;
                }
                ["--quality", quality, tail @ ..] => {
                    options.quality = quality.parse()?;
                    arguments = tail;
                }
                #[cfg(target_os = "linux")]
                ["--device", path, tail @ ..] => {
                    options.device = path.into();
                    arguments = tail;
                }
                _ => return Err("unknown test source option; see --help".into()),
            }
        }
        CaptureSettings::new(options.mode, options.quality, false)?;
        Ok(options)
    }
}

fn serve(address: SocketAddrV4, enable_camera: bool, device: &Path) -> Result<()> {
    if address.ip().is_unspecified() || address.ip().is_multicast() || address.ip().is_broadcast() {
        return Err("bind a specific unicast IPv4 address".into());
    }
    let listener = TcpListener::bind(address)?;
    let SocketAddr::V4(address) = listener.local_addr()? else {
        unreachable!()
    };
    let pairing = Pairing::new(address)?;
    let mut publisher = if enable_camera {
        Some(CameraPublisher::start(device)?)
    } else {
        None
    };
    println!("Paste this one-time invitation into Droid Webcam and tap Start within 3 minutes.");
    println!("{}", *pairing.invite().code());
    println!("Press Enter here to stop.");
    io::stdout().flush()?;
    let stop = stop_on_enter();
    let feed = Feed::default();
    thread::scope(|scope| {
        let worker = scope.spawn(|| host::receive(listener, pairing, stop.clone(), &feed, report));
        if let Some(publisher) = &mut publisher {
            while !worker.is_finished() {
                if let Err(error) = publisher.tick(&feed) {
                    stop.store(true, Ordering::Relaxed);
                    return Err(error);
                }
                thread::sleep(Duration::from_millis(2));
            }
            publisher.tick(&feed)?;
        }
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

fn send_test(options: TestOptions) -> Result<()> {
    println!("Paste the invitation, then press Enter.");
    let mut code = Zeroizing::new(String::new());
    io::stdin().read_line(&mut code)?;
    let invite = Invite::parse(&code)?;
    let settings = CaptureSettings::new(options.mode, options.quality, false)?;
    let mut sender = Sender::connect_with_settings(&invite, settings)?;
    for index in 0..options.count {
        let start = Instant::now();
        sender.send(&synthetic_jpeg(
            (index % options.mode.width()) * 8,
            options.mode,
            options.quality,
        )?)?;
        thread::sleep(options.mode.period().saturating_sub(start.elapsed()));
    }
    sender.stop()
}

#[cfg(windows)]
struct CameraPublisher {
    _camera: windows_host::Camera,
    pipe: droid_camera_source::pipe::Publisher,
    last_frame: Option<Instant>,
}

#[cfg(windows)]
impl CameraPublisher {
    fn start(_: &Path) -> Result<Self> {
        let pipe = droid_camera_source::pipe::Publisher::open()?;
        let camera = windows_host::Camera::start()?;
        println!("Select Droid Webcam in the Windows camera application.");
        Ok(Self {
            pipe,
            _camera: camera,
            last_frame: None,
        })
    }

    fn tick(&mut self, feed: &Feed) -> Result<()> {
        let frame = feed.latest();
        let fresh = frame
            .as_ref()
            .filter(|frame| self.last_frame != Some(frame.captured));
        if let Some(frame) = fresh {
            if self.pipe.tick(Some((
                frame.mode,
                &frame.pixels,
                frame.captured.elapsed().as_millis() as u64,
            )))? {
                self.last_frame = Some(frame.captured);
            }
        } else {
            self.pipe.tick(None)?;
        }
        Ok(())
    }
}

fn test_pattern(options: TestOptions) -> Result<()> {
    let mut publisher = CameraPublisher::start(&options.device)?;
    println!(
        "Showing moving color bars at {}. Press Enter to stop.",
        options.mode
    );
    let stop = stop_on_enter();
    let feed = Feed::default();
    let mut index = 0;
    while !stop.load(Ordering::Relaxed) {
        let start = Instant::now();
        feed.publish(
            options.mode,
            droid_webcam::video::decode_nv12(
                &synthetic_jpeg(index, options.mode, options.quality)?,
                options.mode,
            )?,
        )?;
        publisher.tick(&feed)?;
        index = (index + 8) % options.mode.width();
        thread::sleep(options.mode.period().saturating_sub(start.elapsed()));
    }
    Ok(())
}
