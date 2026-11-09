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
    let (model,user)=load(&session);println!("OT probe ready: {} instances",model.instance_count());
    let script=model.dom().descendants().find(|i|i.class.as_str()=="ModuleScript" && i.name=="rustudio_script_probe" && matches!(i.properties.get(&"Attributes".into()),Some(rbx_types::Variant::Attributes(a)) if matches!(a.get("RustudioSyncProbe"),Some(rbx_types::Variant::Bool(true)))) && model.dom().get_by_ref(i.parent()).is_some_and(|p| p.name=="rustudio_tc_test" && p.class.as_str()=="Folder")).expect("marked test script");
    let guid=match script.properties.get(&"ScriptGuid".into()) {Some(rbx_types::Variant::String(g))=>g.clone(),_=>panic!("script GUID unavailable")};
    let svc=model.dom().descendants().find(|i|i.class.as_str()=="LiveScriptingService").expect("live scripting service");
    let service=model.server_id(svc.referent()).unwrap();
    session.send_event(service,"LiveScriptingService".into(),"LiveEditingClientConfiguration".into(),vec![PropValue::String("{\"version\":\"1\"}".into())]).unwrap();
    session.send_event(service,"LiveScriptingService".into(),"BeginEdit".into(),vec![PropValue::Int(user as i64),PropValue::String(guid)]).unwrap();
    println!("BeginEdit queued for marked test script");
    let until=Instant::now()+Duration::from_secs(45);
    while Instant::now()<until {
        match session.events.recv_timeout(Duration::from_millis(100)) {
            Ok(LiveEvent::Batch(ops))=>for op in ops {if let RepOp::Event {id,name,args}=op {if id==service {println!("live scripting event {name}: {args:?}");}}},
            Ok(LiveEvent::Error(e) | LiveEvent::EditRejected(e)|LiveEvent::Disconnected(e)|LiveEvent::Notice(e))=>println!("live: {e}"),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)=>break,
            _=>{}
        }
    }
    let _=session.send_event(service,"LiveScriptingService".into(),"EndEdit".into(),vec![PropValue::Int(user as i64)]);
    std::thread::sleep(Duration::from_millis(250));
}
