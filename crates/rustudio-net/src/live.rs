//! A cancellable Team Create connection for the editor.
//!
//! Call `LiveSession::connect` only in response to the user's live-open action.
//! The four embedded Studio 0.746 templates contain flag names/build constants,
//! never player JSON, tickets, cookies, or a captured shared-string cache.
//!
//! `Ready` means schema + initial instances + marker have arrived, followed by
//! 1.5 seconds without another decoded initial-instance batch. This is a
//! quiescence heuristic, not a protocol end-of-transfer acknowledgment. Updates
//! continue after `Ready`; callers must continue draining `events`.

use crate::error::{NetError, Result};
use crate::handshake::{self, ProtocolSyncTemplate, TicketTemplate};
use crate::matchmaker::{LiveJoinConsent, Matchmaker, PollBudget};
use crate::replicator::{packet_id, Decoder, PropValue, RepOp, Schema};
use crate::rtcio::{self, lane_tag, CH_CONTROL0, CH_PING, CH_REPLICATION, TAG_CTRL};
use crate::sessioncrypto::Hello;
use crate::transport::{connect_quic, QuicTarget};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const REP_CONTROL: [u8; 7] = [0x83, 0x23, 0, 0, 0, 0, 0];
const BATCH_SIZE: usize = 512;
const MAX_FRAME_SIZE: usize = 64 << 20;
const QUIET_PERIOD: Duration = Duration::from_millis(1500);

#[derive(Debug)]
pub enum LiveEvent {
    /// Join progress, for the joining dialog: a step starting.
    Stage(JoinStage),
    /// The matchmaker is still preparing the server: (attempt, its message).
    Waiting(u32, String),
    Batch(Vec<RepOp>),
    Ready,
    Identity(u64),
    Notice(String),
    /// A persistent document edit was not transmitted; saving must stay blocked.
    EditRejected(String),
    Error(String),
    Disconnected(String),
}

/// The steps of a join, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JoinStage {
    SignIn,
    Matchmaking,
    Connecting,
    Handshake,
    Receiving,
}

impl JoinStage {
    pub const ALL: [JoinStage; 5] = [
        JoinStage::SignIn,
        JoinStage::Matchmaking,
        JoinStage::Connecting,
        JoinStage::Handshake,
        JoinStage::Receiving,
    ];
    pub fn label(self) -> &'static str {
        match self {
            JoinStage::SignIn => "Checking your sign-in",
            JoinStage::Matchmaking => "Asking Roblox for the Team Create server",
            JoinStage::Connecting => "Connecting to the server",
            JoinStage::Handshake => "Joining the session",
            JoinStage::Receiving => "Downloading the place",
        }
    }
}

/// Owns the lifetime of the live connection. Dropping it cancels the
/// worker; drop never joins a network thread or blocks the UI. An already
/// running synchronous account/matchmaker request can finish before noticing
/// cancellation, but cancellation prevents subsequent connection work.
enum LiveEdit {
    Event {id: u64, class: String, name: String, args: Vec<PropValue>},
    Property {
        id: u64,
        class: String,
        name: String,
        value: PropValue,
    },
    Create {
        id: u64,
        class: String,
        parent: u64,
        props: Vec<(String, PropValue)>,
    },
    Delete(u64),
    Reparent {
        id: u64,
        parent: u64,
    },
}

pub struct LiveSession {
    pub events: mpsc::Receiver<LiveEvent>,
    cancel: Arc<AtomicBool>,
    commands: mpsc::SyncSender<LiveEdit>,
    next_id: Arc<AtomicU64>,
}

impl LiveSession {
    pub fn connect(universe: u64, place: u64) -> Result<Self> {
        if universe == 0 || place == 0 {
            return Err(NetError::Other("A universe and place are required".into()));
        }
        // Bound pending model work: a large place cannot enqueue its entire
        // DataModel while the editor is applying a previous batch.
        let (tx, events) = mpsc::sync_channel(8);
        let (commands, command_rx) = mpsc::sync_channel(256);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let next_id = Arc::new(AtomicU64::new(0));
        let worker_id = next_id.clone();
        std::thread::Builder::new()
            .name("rustudio-team-create".into())
            .spawn(move || {
                let result = prepare_and_run(
                    universe,
                    place,
                    &tx,
                    worker_cancel.clone(),
                    command_rx,
                    worker_id,
                );
                if let Err(error) = result {
                    if !worker_cancel.load(Ordering::Acquire) {
                        // Errors created by this module contain fixed context,
                        // never response bodies or secret-bearing packet bytes.
                        let _ = tx.send(LiveEvent::Error(error.to_string()));
                    }
                }
            })
            .map_err(|_| NetError::Other("Could not start Team Create worker".into()))?;
        Ok(Self {
            events,
            cancel,
            commands,
            next_id,
        })
    }
    pub fn send_property(
        &self,
        id: u64,
        class: String,
        name: String,
        value: PropValue,
    ) -> Result<()> {
        self.commands
            .try_send(LiveEdit::Property {
                id,
                class,
                name,
                value,
            })
            .map_err(|_| NetError::Other("Team Create edit queue is unavailable".into()))
    }
    pub fn send_event(&self, id: u64, class: String, name: String, args: Vec<PropValue>) -> Result<()> {
        self.send_edit(LiveEdit::Event {id,class,name,args})
    }
    pub fn allocate_id(&self) -> Result<u64> {
        self.next_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| {
                (id != 0 && id as u32 != u32::MAX).then_some(id + 1)
            })
            .map_err(|_| NetError::Other("Team Create instance namespace is unavailable".into()))
    }
    fn send_edit(&self, edit: LiveEdit) -> Result<()> {
        self.commands
            .try_send(edit)
            .map_err(|_| NetError::Other("Team Create edit queue is unavailable".into()))
    }
    pub fn send_create(
        &self,
        id: u64,
        class: String,
        parent: u64,
        props: Vec<(String, PropValue)>,
    ) -> Result<()> {
        self.send_edit(LiveEdit::Create {
            id,
            class,
            parent,
            props,
        })
    }
    pub fn send_delete(&self, id: u64) -> Result<()> {
        self.send_edit(LiveEdit::Delete(id))
    }
    pub fn send_reparent(&self, id: u64, parent: u64) -> Result<()> {
        self.send_edit(LiveEdit::Reparent { id, parent })
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

fn templates() -> (ProtocolSyncTemplate, TicketTemplate) {
    (
        ProtocolSyncTemplate {
            head: include_bytes!("templates/studio_0746_sync_head.bin").to_vec(),
            tail: include_bytes!("templates/studio_0746_sync_tail.bin").to_vec(),
        },
        TicketTemplate {
            middle: include_bytes!("templates/studio_0746_ticket_middle.bin").to_vec(),
            tail: include_bytes!("templates/studio_0746_ticket_tail.bin").to_vec(),
        },
    )
}

fn prepare_and_run(
    universe: u64,
    place: u64,
    tx: &mpsc::SyncSender<LiveEvent>,
    cancel: Arc<AtomicBool>,
    commands: mpsc::Receiver<LiveEdit>,
    next_id: Arc<AtomicU64>,
) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let _ = tx.send(LiveEvent::Stage(JoinStage::SignIn));
    let (cookie, _) = rustudio_account::session::load_cookie(false)
        .map_err(|_| NetError::Other("Could not load your Roblox sign-in".into()))?
        .ok_or_else(|| NetError::Other("Sign in to Roblox before joining Team Create".into()))?;
    let session = rustudio_account::Session::validate(cookie)
        .map_err(|_| NetError::Other("Your Roblox sign-in could not be validated".into()))?;
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let _ = tx.send(LiveEvent::Identity(session.profile.id));
    let _ = tx.send(LiveEvent::Stage(JoinStage::Matchmaking));
    let joined = Matchmaker::new(&session)
        .join_with_progress(
            universe,
            place,
            LiveJoinConsent::user_requested_live_join(),
            PollBudget::default(),
            &mut |attempt, message| {
                if attempt > 1 {
                    let _ = tx.send(LiveEvent::Waiting(attempt, message.to_string()));
                }
            },
        )
        .map_err(|error| match error {
            NetError::Http { status, .. } => {
                NetError::Other(format!("Team Create request returned HTTP {status}"))
            }
            NetError::CannotCollaborate(_) => {
                NetError::Other("This account cannot collaborate on this universe".into())
            }
            NetError::MatchmakerTimeout => NetError::MatchmakerTimeout,
            _ => NetError::Other("Roblox could not prepare this Team Create session".into()),
        })?;
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let raw: serde_json::Value = serde_json::from_str(joined.raw.expose())
        .map_err(|_| NetError::Other("Roblox returned invalid join information".into()))?;
    let target = QuicTarget::from_join(&joined.join)
        .map_err(|_| NetError::Other("Roblox returned unsupported transport information".into()))?;
    let (sync, ticket) = templates();
    let opening = vec![
        Hello::from_client_ticket(joined.join.client_ticket.expose())
            .and_then(|hello| hello.encode())
            .map_err(|_| NetError::Other("Roblox returned an unsupported session ticket".into()))?,
        handshake::shared_string_cache_message(&[])?,
        handshake::protocol_sync(&sync, &raw),
        handshake::fresh_place_verification()?,
        handshake::submit_ticket(&ticket, &joined.join, &raw),
        vec![0x8f, 0],
    ];
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| NetError::Other("Could not start Team Create network runtime".into()))?;
    runtime
        .block_on(run(target, opening, tx.clone(), cancel, commands, next_id))
        .map_err(|_| NetError::Other("The Team Create network session failed".into()))
}

#[derive(Default)]
struct InitialState {
    schema: bool,
    marker: bool,
    instances: bool,
    last_batch: Option<Instant>,
    delivering: bool,
}

async fn emit(tx: &mpsc::SyncSender<LiveEvent>, cancel: &AtomicBool, mut event: LiveEvent) -> bool {
    loop {
        if cancel.load(Ordering::Acquire) {
            return false;
        }
        match tx.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
            Err(mpsc::TrySendError::Full(value)) => event = value,
        }
        // Yield so QUIC keeps servicing ACKs while the UI catches up.
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn read_frame(stream: &mut quinn::RecvStream) -> Result<Vec<u8>> {
    let mut length = [0; 4];
    stream
        .read_exact(&mut length)
        .await
        .map_err(|e| NetError::Other(format!("The replication stream ended: {e}")))?;
    let word = u32::from_be_bytes(length);
    let size = (word & 0x7fff_ffff) as usize;
    if word & 0x8000_0000 == 0 || size > MAX_FRAME_SIZE {
        return Err(NetError::Other(
            "The server sent an unsupported replication frame".into(),
        ));
    }
    let mut frame = vec![0; size];
    stream
        .read_exact(&mut frame)
        .await
        .map_err(|_| NetError::Other("The replication stream ended during a frame".into()))?;
    Ok(frame)
}

async fn receive_lane(
    mut stream: quinn::RecvStream,
    tx: mpsc::SyncSender<LiveEvent>,
    cancel: Arc<AtomicBool>,
    seen_sync: Arc<AtomicBool>,
    initial: Arc<Mutex<InitialState>>,
    lane_ended: Arc<AtomicBool>,
    shared_schema: Arc<Mutex<Option<Schema>>>,
    next_id: Arc<AtomicU64>,
) {
    let Some((channel, tag)) = rtcio::read_header(&mut stream).await else {
        return;
    };
    let primary = channel == CH_REPLICATION && tag == lane_tag(1);
    let mut decoder = Decoder::new();
    let mut schema_frame = Vec::new();
    let mut capture = if primary {
        std::env::var("RUSTUDIO_TC_STREAM_OUT")
            .ok()
            .and_then(|path| {
                use std::io::Write;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create(true).truncate(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut f = options.open(path).ok()?;
                f.write_all(&[6, 1, 4, 0, 0, 0, 1]).ok()?;
                Some(f)
            })
    } else {
        None
    };
    loop {
        let frame = match read_frame(&mut stream).await {
            Ok(frame) => frame,
            Err(error) => {
                if primary {
                    emit(&tx, &cancel, LiveEvent::Disconnected(error.to_string())).await;
                    lane_ended.store(true, Ordering::Release);
                }
                return;
            }
        };
        if cancel.load(Ordering::Acquire) {
            return;
        }
        if !primary || frame.is_empty() {
            continue;
        }
        if let Some(file) = capture.as_mut() {
            use std::io::Write;
            let _ = file.write_all(&rtcio::frame(&frame));
        }
        if frame[0] == packet_id::SET_GLOBALS {
            match crate::replicator::assigned_peer_scope(&frame[1..]) {
                Ok(scope) => {
                    next_id.store((u64::from(scope) << 32) | 1, Ordering::Release);
                }
                Err(_) => {
                    emit(
                        &tx,
                        &cancel,
                        LiveEvent::Notice(
                            "Instance creation is unavailable for this server configuration".into(),
                        ),
                    )
                    .await;
                }
            }
        }
        if frame[0] == packet_id::PROTOCOL_SYNC {
            seen_sync.store(true, Ordering::Release);
        }
        initial.lock().unwrap().delivering = true;
        if frame[0] == packet_id::NEW_SCHEMA {
            schema_frame = frame.clone();
        }
        let errors = decoder.errors.len();
        let ops = decoder.decode_frame(frame[0], &frame[1..]);
        if decoder.errors.len() != errors {
            eprintln!("Team Create decode error: {:?}", decoder.errors.last());
            if let Ok(path) = std::env::var("RUSTUDIO_TC_BAD_FRAME") {
                use std::io::Write;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create(true).truncate(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                if let Ok(mut file) = options.open(path) {
                    let _ = file.write_all(&[6, 1, 4, 0, 0, 0, 1]);
                    let _ = file.write_all(&rtcio::frame(&schema_frame));
                    let _ = file.write_all(&rtcio::frame(&frame));
                }
            }
            emit(
                &tx,
                &cancel,
                LiveEvent::Error(
                    "Some live place data could not be decoded; the session was stopped".into(),
                ),
            )
            .await;
            lane_ended.store(true, Ordering::Release);
            return;
        }
        if frame[0] == packet_id::NEW_SCHEMA {
            *shared_schema.lock().unwrap() = decoder.schema.clone();
        }
        let instances = ops.iter().any(|op| matches!(op, RepOp::NewInstance { .. }));
        let mut pending = Vec::with_capacity(BATCH_SIZE);
        for op in ops {
            pending.push(op);
            if pending.len() == BATCH_SIZE {
                if !emit(
                    &tx,
                    &cancel,
                    LiveEvent::Batch(std::mem::replace(
                        &mut pending,
                        Vec::with_capacity(BATCH_SIZE),
                    )),
                )
                .await
                {
                    return;
                }
            }
        }
        if !pending.is_empty() && !emit(&tx, &cancel, LiveEvent::Batch(pending)).await {
            return;
        }
        let mut state = initial.lock().unwrap();
        state.schema |= decoder.schema.is_some();
        state.marker |= frame[0] == packet_id::MARKER;
        if instances {
            state.instances = true;
            state.last_batch = Some(Instant::now());
        }
        state.delivering = false;
    }
}

async fn wait_cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn run(
    target: QuicTarget,
    opening: Vec<Vec<u8>>,
    tx: mpsc::SyncSender<LiveEvent>,
    cancel: Arc<AtomicBool>,
    commands: mpsc::Receiver<LiveEdit>,
    next_id: Arc<AtomicU64>,
) -> Result<()> {
    emit(&tx, &cancel, LiveEvent::Stage(JoinStage::Connecting)).await;
    let (_endpoint, connection, _socket) = tokio::select! {
        connected = connect_quic(&target) => connected
            .map_err(|_| NetError::Other("Could not connect to the Team Create server".into()))?,
        _ = wait_cancelled(&cancel) => return Ok(()),
    };
    if cancel.load(Ordering::Acquire) {
        connection.close(0u32.into(), b"cancelled");
        return Ok(());
    }
    let shared_schema = Arc::new(Mutex::new(None::<Schema>));
    let seen_sync = Arc::new(AtomicBool::new(false));
    let lane_ended = Arc::new(AtomicBool::new(false));
    let initial = Arc::new(Mutex::new(InitialState::default()));
    let mut held_lanes = vec![rtcio::open_lane(&connection, CH_REPLICATION, lane_tag(0)).await?];
    let mut replication = rtcio::open_lane(&connection, CH_REPLICATION, lane_tag(1)).await?;
    for seq in [2, 4, 7, 8, 9, 10, 12, 13] {
        held_lanes.push(rtcio::open_lane(&connection, CH_REPLICATION, lane_tag(seq)).await?);
    }
    let mut replication_control = rtcio::open_lane(&connection, CH_REPLICATION, TAG_CTRL).await?;
    rtcio::send(&mut replication_control, &[0, 0]).await?;
    for (lane, stream) in [(3u32, 2u32), (5, 4), (6, 6), (11, 8)] {
        let mut msg = vec![2, CH_REPLICATION];
        msg.extend_from_slice(&lane.to_be_bytes());
        msg.extend_from_slice(&stream.to_be_bytes());
        rtcio::send(&mut replication_control, &msg).await?;
    }
    let mut ping_control = rtcio::open_lane(&connection, CH_PING, TAG_CTRL).await?;
    rtcio::send(&mut ping_control, &[0, 0]).await?;
    for (lane, stream) in [(2u32, 10u32), (3, 12)] {
        let mut msg = vec![2, CH_PING];
        msg.extend_from_slice(&lane.to_be_bytes());
        msg.extend_from_slice(&stream.to_be_bytes());
        rtcio::send(&mut ping_control, &msg).await?;
    }
    let mut zero_control = rtcio::open_lane(&connection, CH_CONTROL0, TAG_CTRL).await?;
    rtcio::send(&mut zero_control, &[0, 0]).await?;
    let mut ping = rtcio::open_lane(&connection, CH_PING, lane_tag(1)).await?;
    emit(&tx, &cancel, LiveEvent::Stage(JoinStage::Handshake)).await;
    for message in opening {
        rtcio::send(&mut replication, &message).await?;
    }
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_ping = Instant::now();
    let mut control_started = false;
    let mut ready = false;
    let mut rejected_document_edit = false;
    loop {
        tokio::select! {
            incoming = connection.accept_uni() => {
                match incoming {
                    Ok(stream) => {
                        tokio::spawn(receive_lane(stream, tx.clone(), cancel.clone(), seen_sync.clone(), initial.clone(), lane_ended.clone(), shared_schema.clone(), next_id.clone()));
                    }
                    Err(_) => {
                        emit(&tx, &cancel, LiveEvent::Disconnected("The Team Create connection ended".into())).await;
                        break;
                    }
                }
            }
            _ = interval.tick() => {
                if cancel.load(Ordering::Acquire) || lane_ended.load(Ordering::Acquire) { break; }
                if seen_sync.swap(false, Ordering::AcqRel) {
                    rtcio::send(&mut replication, &REP_CONTROL).await?;
                    control_started = true;
                }
                for _ in 0..64 {
                    let Ok(edit) = commands.try_recv() else { break; };
                    let document_edit=matches!(&edit,LiveEdit::Create {..}|LiveEdit::Delete(_)|LiveEdit::Reparent {..})
                        || matches!(&edit,LiveEdit::Property {class,..} if class!="Player");
                    if rejected_document_edit && matches!(&edit,LiveEdit::Event {class,name,..} if class=="TeamCreatePublishService" && name=="TeamCreatePlaceUploadRequest") {
                        emit(&tx,&cancel,LiveEvent::EditRejected("Save blocked because a local edit could not be synchronized; retain a local copy and reopen the live place".into())).await;
                        continue;
                    }
                    let encoded = {
                        let schema = shared_schema.lock().unwrap();
                        schema.as_ref().ok_or_else(|| "The live schema is not ready".to_string())
                            .and_then(|schema| match edit {
                                LiveEdit::Event {id,class,name,args} => crate::writer::event(schema,id,&class,&name,&args),
                                LiveEdit::Property {id, class, name, value} => crate::writer::property(schema, id, &class, &name, &value),
                                LiveEdit::Create {id, class, parent, props} => crate::writer::create(schema, id, &class, parent, &props),
                                LiveEdit::Delete(id) => Ok(crate::writer::delete(id)),
                                LiveEdit::Reparent {id, parent} => crate::writer::reparent(schema, id, parent),
                            })
                    };
                    match encoded {
                        Ok(frame) => rtcio::send(&mut replication, &frame).await?,
                        Err(error) => {
                            if document_edit {rejected_document_edit=true;emit(&tx,&cancel,LiveEvent::EditRejected(error)).await;}
                            else {emit(&tx, &cancel, LiveEvent::Notice(error)).await;}
                        }
                    }
                }
                if last_ping.elapsed() >= Duration::from_secs(1) {
                    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_micros() as u64;
                    rtcio::send(&mut ping, &rtcio::ping_message(timestamp)).await?;
                    if control_started { rtcio::send(&mut replication, &REP_CONTROL).await?; }
                    last_ping = Instant::now();
                }
                let quiet = {
                    let state = initial.lock().unwrap();
                    state.schema && state.marker && state.instances && !state.delivering
                        && state.last_batch.is_some_and(|at| at.elapsed() >= QUIET_PERIOD)
                };
                if !ready && quiet {
                    if !emit(&tx, &cancel, LiveEvent::Ready).await { break; }
                    ready = true;
                }
            }
        }
    }
    connection.close(0u32.into(), b"editor closed session");
    Ok(())
}
