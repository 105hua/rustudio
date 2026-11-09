//! Cross-account synchronization probe. Only use in an authorized test place.
//! Creates a temporary Folder beneath Workspace.rustudio_tc_test.
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
        *pos = [123.25, 456.5, 789.75];
    }
    first
        .send_property(
            id,
            "Player".into(),
            "CloudEditCameraCoordinateFrame".into(),
            update,
        )
        .unwrap();
    println!("camera sent: 123.25,456.5,789.75");
    let folder = model
        .dom()
        .descendants()
        .find(|i| i.class.as_str() == "Folder" && i.name == "rustudio_tc_test")
        .expect("authorized test folder");
    let parent = model.server_id(folder.referent()).unwrap();
    let created = first.allocate_id().expect("assigned client namespace");
    println!("allocated test instance scope {}", created >> 32);
    first
        .send_create(
            created,
            "Folder".into(),
            parent,
            vec![(
                "Name".into(),
                PropValue::String("rustudio_outgoing_probe".into()),
            )],
        )
        .unwrap();
    println!("outgoing folder creation sent");
    let until = Instant::now() + Duration::from_secs(180);
    let mut sent_probe = false;
    let mut probe_id = None;
    while Instant::now() < until {
        match first.events.recv_timeout(Duration::from_millis(100)) {
            Ok(LiveEvent::Batch(ops)) => {
                for op in ops {
                    if let RepOp::ChangeProperty {
                        id,
                        ref name,
                        ref value,
                    } = op
                    {
                        if Some(id) == probe_id {
                            println!("incoming probe property {name}: {value:?}");
                        }
                    }
                    if let RepOp::NewInstance {
                        id,
                        ref class,
                        ref props,
                        ..
                    } = op
                    {
                        if class == "Part"
                            && props.iter().any(|(k, v)| {
                                k == "Name" && v == &PropValue::String("rustudio_sync_probe".into())
                            })
                            && !sent_probe
                        {
                            println!("received peer-created probe {id}");
                            first
                                .send_property(
                                    id,
                                    "Part".into(),
                                    "Name".into(),
                                    PropValue::String("rustudio_sync_probe_ack".into()),
                                )
                                .unwrap();
                            first
                                .send_property(
                                    id,
                                    "Part".into(),
                                    "CFrame".into(),
                                    PropValue::CFrame {
                                        pos: [12345.0, 12345.0, 12345.0],
                                        rot: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                                    },
                                )
                                .unwrap();
                            first.send_reparent(id, created).unwrap();
                            println!("probe edits and reparent sent");
                            sent_probe = true;
                            probe_id = Some(id);
                        }
                    }
                }
            }
            Ok(LiveEvent::Error(e) | LiveEvent::EditRejected(e) | LiveEvent::Disconnected(e) | LiveEvent::Notice(e)) => {
                println!("live: {e}")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
    }
    let _ = first.send_delete(created);
    std::thread::sleep(Duration::from_millis(500));
    println!("outgoing test folder deletion sent");
    let _ = first.send_property(
        id,
        "Player".into(),
        "CloudEditCameraCoordinateFrame".into(),
        original,
    );
}
