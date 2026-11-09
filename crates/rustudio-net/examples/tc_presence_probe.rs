//! Opt-in cross-account presence probe in the authorized marked test subtree.
use rustudio_net::{
    live::{LiveEvent, LiveSession},
    model::LiveModel,
    replicator::{PropValue, RepOp},
};
use std::time::{Duration, Instant};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let session = LiveSession::connect(args[0].parse().unwrap(), args[1].parse().unwrap()).unwrap();
    let mut model = LiveModel::new();
    let mut user = 0;
    let until = Instant::now() + Duration::from_secs(90);
    loop {
        assert!(Instant::now() < until, "join timed out");
        match session.events.recv_timeout(Duration::from_secs(1)) {
            Ok(LiveEvent::Identity(id)) => user = id,
            Ok(LiveEvent::Stage(_) | LiveEvent::Waiting(..)) => {}
            Ok(LiveEvent::Batch(ops)) => {
                for op in ops {
                    model.apply(op);
                }
            }
            Ok(LiveEvent::Ready) => break,
            Ok(LiveEvent::Error(e) | LiveEvent::EditRejected(e) | LiveEvent::Disconnected(e)) => {
                panic!("{e}")
            }
            _ => {}
        }
    }
    let script=model.dom().descendants().find(|i|i.class.as_str()=="ModuleScript" && i.name=="rustudio_script_probe"
        && model.dom().get_by_ref(i.parent()).is_some_and(|p|p.name=="rustudio_tc_test")
        && matches!(i.properties.get(&"Attributes".into()),Some(rbx_types::Variant::Attributes(a)) if matches!(a.get("RustudioSyncProbe"),Some(rbx_types::Variant::Bool(true)))))
        .and_then(|i|model.server_id(i.referent())).expect("marked test script");
    let player=model.dom().descendants().find(|i|i.class.as_str()=="Player" && matches!(i.properties.get(&"UserId".into()),Some(rbx_types::Variant::Int64(id)) if *id as u64==user))
        .and_then(|i|model.server_id(i.referent())).expect("own player");
    session
        .send_event(
            player,
            "Player".into(),
            "CloudEditSelectionChanged".into(),
            vec![PropValue::Array(vec![PropValue::Ref(Some(script))])],
        )
        .unwrap();
    session
        .send_property(
            player,
            "Player".into(),
            "CloudEditPlayerActive".into(),
            PropValue::Bool(true),
        )
        .unwrap();
    println!("presence ready; marked test selection and active state queued");
    let until = Instant::now() + Duration::from_secs(30);
    let mut received = false;
    let mut cleared = false;
    while Instant::now() < until {
        if !cleared && until.saturating_duration_since(Instant::now()) < Duration::from_secs(27) {
            session
                .send_event(
                    player,
                    "Player".into(),
                    "CloudEditSelectionChanged".into(),
                    vec![PropValue::Array(vec![])],
                )
                .unwrap();
            cleared = true;
        }
        match session.events.recv_timeout(Duration::from_millis(100)) {
            Ok(LiveEvent::Batch(ops)) => {
                for op in ops {
                    if let RepOp::Event { id, name, args } = op {
                        if id != player && name == "Player.CloudEditSelectionChanged" {
                            if let Some(PropValue::Array(values)) = args.first() {
                                if values.contains(&PropValue::Ref(Some(script))) {
                                    received = true;
                                    println!("peer selection of marked test script received");
                                }
                            }
                        }
                    }
                }
            }
            Ok(
                LiveEvent::Error(e)
                | LiveEvent::EditRejected(e)
                | LiveEvent::Disconnected(e)
                | LiveEvent::Notice(e),
            ) => panic!("{e}"),
            _ => {}
        }
    }
    assert!(received, "peer selection was not received");
    println!("cross-account selection presence passed");
}
