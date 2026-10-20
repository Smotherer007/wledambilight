use std::io::Read;
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::{CreateSessionOptions, PersistMode};
use ashpd::enumflags2::BitFlags;

#[derive(Debug, Clone, PartialEq)]
pub enum CaptureStatus {
    Idle,
    Starting,
    Running { w: u32, h: u32 },
    Error(String),
}

#[derive(Clone)]
pub struct SharedFrame {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
    pub seq: u64,
}

impl SharedFrame {
    pub fn black(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            data: vec![0; (w * h * 3) as usize],
            seq: 0,
        }
    }
}

pub type FrameSlot = Arc<Mutex<SharedFrame>>;

pub struct ScreenCapture {
    pub slot: FrameSlot,
    pub screen: Arc<Mutex<(u32, u32)>>,
    pub status: Arc<Mutex<CaptureStatus>>,
    pub restore_token: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct PortalStream {
    node: u32,
    size: (i32, i32),
    token: Option<String>,
    fd: std::os::fd::OwnedFd,
    _session: ashpd::desktop::Session<Screencast>,
}

async fn open_stream(restore: Option<String>) -> Result<PortalStream, String> {
    let proxy = Screencast::new().await.map_err(|e| format!("portal: {e}"))?;
    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|e| format!("session: {e}"))?;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_sources(BitFlags::from(SourceType::Monitor))
                .set_multiple(false)
                .set_cursor_mode(CursorMode::Hidden)
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(restore.as_deref()),
        )
        .await
        .map_err(|e| format!("select sources: {e}"))?;
    let request = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|e| format!("start: {e}"))?;
    let streams = request.response().map_err(|e| format!("start refused: {e}"))?;
    let first = streams
        .streams()
        .first()
        .ok_or_else(|| "portal returned no stream".to_string())?
        .clone();
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|e| format!("pipewire remote: {e}"))?;
    Ok(PortalStream {
        node: first.pipe_wire_node_id(),
        size: first.size().unwrap_or((0, 0)),
        token: streams.restore_token().map(|s| s.to_string()),
        fd,
        _session: session,
    })
}

impl ScreenCapture {
    /// Spawn the capture thread. `width` is the horizontal size of the capture
    /// buffer; the height follows the screen aspect and is capped.
    pub fn start(width: u32, restore_token: Option<String>) -> Self {
        let width = width.clamp(64, 3840);
        let slot: FrameSlot = Arc::new(Mutex::new(SharedFrame::black(width, width * 9 / 16)));
        let screen = Arc::new(Mutex::new((0u32, 0u32)));
        let status = Arc::new(Mutex::new(CaptureStatus::Starting));
        let token_out = Arc::new(Mutex::new(restore_token.clone()));
        let stop = Arc::new(AtomicBool::new(false));

        let (s, st, tok, sp) = (
            slot.clone(),
            status.clone(),
            token_out.clone(),
            stop.clone(),
        );
        let scr = screen.clone();
        let join = std::thread::spawn(move || {
            *st.lock().unwrap() = CaptureStatus::Starting;
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    *st.lock().unwrap() = CaptureStatus::Error(format!("runtime: {e}"));
                    return;
                }
            };
            let stream = match rt.block_on(open_stream(restore_token)) {
                Ok(s) => s,
                Err(e) => {
                    *st.lock().unwrap() = CaptureStatus::Error(e);
                    return;
                }
            };
            if let Some(t) = stream.token.clone() {
                *tok.lock().unwrap() = Some(t);
            }
            let (sw, sh) = stream.size;
            if sw > 0 && sh > 0 {
                *scr.lock().unwrap() = (sw as u32, sh as u32);
            }
            let h = if sw > 0 && sh > 0 {
                ((width as f32 * sh as f32 / sw as f32).round() as u32).clamp(32, 2160) & !1
            } else {
                width * 9 / 16
            };

            let pipeline = format!(
                "pipewiresrc fd=3 path={} do-timestamp=true ! videoconvert ! videoscale ! video/x-raw,format=RGB,width={},height={} ! fdsink fd=1",
                stream.node, width, h
            );
            let raw = stream.fd.as_raw_fd();
            let mut cmd = Command::new("gst-launch-1.0");
            cmd.arg("-q");
            for tok in pipeline.split_whitespace() {
                cmd.arg(tok);
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::null());
            unsafe {
                use std::os::unix::process::CommandExt;
                cmd.pre_exec(move || {
                    // die with the parent instead of lingering as an orphan
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    if raw != 3 && libc::dup2(raw, 3) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    let flags = libc::fcntl(3, libc::F_GETFD);
                    libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                    Ok(())
                });
            }
            let mut child: Child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    *st.lock().unwrap() =
                        CaptureStatus::Error(format!("gst-launch-1.0 failed: {e}"));
                    return;
                }
            };
            let Some(mut stdout) = child.stdout.take() else {
                *st.lock().unwrap() = CaptureStatus::Error("no gst stdout".into());
                return;
            };

            {
                let mut g = s.lock().unwrap();
                *g = SharedFrame::black(width, h);
            }
            *st.lock().unwrap() = CaptureStatus::Running { w: width, h };

            let frame_len = (width * h * 3) as usize;
            let mut buf = vec![0u8; frame_len];
            loop {
                if sp.load(Ordering::Relaxed) {
                    break;
                }
                match stdout.read_exact(&mut buf) {
                    Ok(()) => {
                        let mut g = s.lock().unwrap();
                        if g.data.len() == frame_len {
                            g.data.copy_from_slice(&buf);
                            g.seq += 1;
                        }
                    }
                    Err(e) => {
                        if sp.load(Ordering::Relaxed) {
                            break;
                        }
                        *st.lock().unwrap() = CaptureStatus::Error(format!(
                            "capture stopped: {e}. Screen capture may need to be reconnected."
                        ));
                        break;
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            // keep the portal session alive until the very end
            drop(stream.fd);
            drop(stream._session);
            std::thread::sleep(Duration::from_millis(10));
        });

        Self {
            slot,
            screen,
            status,
            restore_token: token_out,
            stop,
            join: Some(join),
        }
    }

    pub fn status(&self) -> CaptureStatus {
        self.status.lock().unwrap().clone()
    }
}
