//! Cross-account synchronization probe. Only use in an authorized test place.
//! Requires the marked ModuleScript beneath Workspace.rustudio_tc_test.
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
    let args:Vec<_>=std::env::args().skip(1).collect();
    let session=LiveSession::connect(args[0].parse().unwrap(),args[1].parse().unwrap()).unwrap();
    let (model,_)=load(&session);println!("source probe ready: {} instances",model.instance_count());
    let until=Instant::now()+Duration::from_secs(180);
    let mut script_id=model.dom().descendants().find(|i| i.class.as_str()=="ModuleScript" && i.name=="rustudio_script_probe" && matches!(i.properties.get(&"Attributes".into()),Some(rbx_types::Variant::Attributes(a)) if matches!(a.get("RustudioSyncProbe"),Some(rbx_types::Variant::Bool(true)))) && model.dom().get_by_ref(i.parent()).is_some_and(|p| p.name=="rustudio_tc_test" && p.class.as_str()=="Folder")).and_then(|i|model.server_id(i.referent()));
    if let Some(id)=script_id {
        session.send_property(id,"ModuleScript".into(),"Source".into(),PropValue::String("-- rustudio synchronization probe\nreturn 3\n".into())).unwrap();
        println!("outgoing marked ModuleScript source queued");
    }

    while Instant::now()<until {
        match session.events.recv_timeout(Duration::from_millis(100)) {
            Ok(LiveEvent::Batch(ops))=>for op in ops {
                match op {
                    RepOp::NewInstance {id,class,props,..} if class=="ModuleScript" && props.iter().any(|(k,v)| k=="Name" && v==&PropValue::String("rustudio_script_probe".into())) => {
                        script_id=Some(id);println!("received marked ModuleScript {id}");
                        for (key,value) in props {if key=="Source" {if let PropValue::CachedSharedString {hash,bytes}=value {
                            println!("probe Source bytes {}, BLAKE2b-128 matches {}",bytes.len(),{use blake2::{Blake2b,Digest,digest::consts::U16};hash.as_slice()==Blake2b::<U16>::digest(&bytes).as_slice()});
                        }}}
                    }
                    RepOp::ChangeProperty {id,name,value} if Some(id)==script_id && name=="Source" => println!("received source update: {value:?}"),
                    _=>{}
                }
            },
            Ok(LiveEvent::Error(e) | LiveEvent::EditRejected(e)|LiveEvent::Disconnected(e)|LiveEvent::Notice(e))=>println!("live: {e}"),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)=>break,
            _=>{}
        }
    }
}
