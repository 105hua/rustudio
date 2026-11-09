//! Verify outgoing replication using only our own collaborator camera.
//! Opens two authorized live sessions; no place instances are edited.
use rustudio_net::{
    live::{LiveEvent, LiveSession},
    model::LiveModel,
    replicator::{PropValue, RepOp},
};
use std::time::{Duration, Instant};
fn load(session: &LiveSession) -> (LiveModel, u64) {
    let mut model = LiveModel::new();
    let mut user = 0;
    let until = Instant::now() + Duration::from_secs(90);
    while Instant::now() < until {
        match session.events.recv_timeout(Duration::from_secs(1)) {
            Ok(LiveEvent::Identity(id)) => user = id,
            Ok(LiveEvent::Stage(_) | LiveEvent::Waiting(..)) => {}
            Ok(LiveEvent::Batch(ops)) => {
                for op in ops {
                    model.apply(op);
                }
            }
            Ok(LiveEvent::Ready) => return (model, user),
            Ok(LiveEvent::Error(e) | LiveEvent::EditRejected(e) | LiveEvent::Disconnected(e)) => panic!("{e}"),
            Ok(LiveEvent::Notice(message)) => println!("notice: {message}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => panic!("event channel: {e}"),
        }
    }
    panic!("initial model timed out")
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let u = args[0].parse().unwrap();
    let p = args[1].parse().unwrap();
    let first = LiveSession::connect(u, p).unwrap();
    let (model, user) = load(&first);
    println!("first ready: {} instances", model.instance_count());
    let own = model.dom().descendants().find(|inst| inst.class.as_str() == "Player" &&
        matches!(inst.properties.get(&"UserId".into()), Some(rbx_types::Variant::Int64(uid)) if *uid as u64 == user))
        .expect("own player");
    let id = model.server_id(own.referent()).unwrap();
    let original = match own.properties.get(&"CloudEditCameraCoordinateFrame".into()) {
        Some(rbx_types::Variant::CFrame(v)) => PropValue::CFrame {
            pos: [v.position.x, v.position.y, v.position.z],
            rot: [
                v.orientation.x.x,
                v.orientation.x.y,
                v.orientation.x.z,
                v.orientation.y.x,
                v.orientation.y.y,
                v.orientation.y.z,
                v.orientation.z.x,
                v.orientation.z.y,
                v.orientation.z.z,
            ],
        },
        None => PropValue::CFrame {
            pos: [0.0; 3],
            rot: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        },
        _ => panic!("camera property type"),
    };
    let mut update = original.clone();
    if let PropValue::CFrame { pos, .. } = &mut update {
        pos[0] += 1.25;
    }
    first
        .send_property(
            id,
            "Player".into(),
            "CloudEditCameraCoordinateFrame".into(),
            update.clone(),
        )
        .unwrap();
    let mut origin_seen = false;
    let sent_until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < sent_until {
        match first.events.recv_timeout(Duration::from_millis(100)) {
            Ok(LiveEvent::Batch(ops)) => {
                for op in ops {
                    if matches!(op, RepOp::ChangeProperty { ref name, ref value, .. } if (name=="CFrame" || name=="CloudEditCameraCoordinateFrame") && value==&update)
                    {
                        origin_seen = true;
                    }
                }
            }
            Ok(LiveEvent::Error(e) | LiveEvent::EditRejected(e) | LiveEvent::Disconnected(e) | LiveEvent::Notice(e)) => {
                println!("origin before observer: {e}")
            }
            _ => {}
        }
    }
    let second = LiveSession::connect(u, p).unwrap();
    let (observer, _) = load(&second);
    println!("observer ready: {} instances", observer.instance_count());
    let initial_seen = observer.dom().descendants().any(|inst| inst.class.as_str()=="Collaborator" &&
        matches!(inst.properties.get(&"CFrame".into()), Some(rbx_types::Variant::CFrame(frame))
            if matches!(&update,PropValue::CFrame{pos,..} if frame.position.x==pos[0] && frame.position.y==pos[1] && frame.position.z==pos[2])));
    let until = Instant::now() + Duration::from_secs(8);
    println!("server reflected camera before observer: {origin_seen}");
    let mut seen = initial_seen || origin_seen;
    while Instant::now() < until {
        // Drain the origin too, so bounded queues do not stall either client.
        while let Ok(event) = first.events.try_recv() {
            if let LiveEvent::Error(e) | LiveEvent::EditRejected(e) | LiveEvent::Disconnected(e) | LiveEvent::Notice(e) = event {
                println!("origin: {e}");
            }
        }
        match second.events.recv_timeout(Duration::from_millis(100)) {
            Ok(LiveEvent::Batch(ops)) => {
                for op in ops {
                    if matches!(op, RepOp::ChangeProperty { id: actual, ref name, ref value } if actual==id && name=="CFrame" && value==&update)
                    {
                        seen = true;
                    }
                }
            }
            Ok(LiveEvent::Error(e) | LiveEvent::EditRejected(e) | LiveEvent::Disconnected(e) | LiveEvent::Notice(e)) => {
                println!("observer: {e}")
            }
            _ => {}
        }
        if seen {
            break;
        }
    }
    let _ = first.send_property(
        id,
        "Player".into(),
        "CloudEditCameraCoordinateFrame".into(),
        original,
    );
    // Allow the restoration to be sent before closing the sessions.
    std::thread::sleep(Duration::from_millis(300));
    println!("collaborator camera observed by second client: {seen}");
    assert!(seen, "server did not relay our camera update");
}
