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
    let (model,_user)=load(&session);println!("OT probe ready: {} instances",model.instance_count());
    let script=model.dom().descendants().find(|i|i.class.as_str()=="ModuleScript" && i.name=="rustudio_script_probe" && matches!(i.properties.get(&"Attributes".into()),Some(rbx_types::Variant::Attributes(a)) if matches!(a.get("RustudioSyncProbe"),Some(rbx_types::Variant::Bool(true)))) && model.dom().get_by_ref(i.parent()).is_some_and(|p| p.name=="rustudio_tc_test" && p.class.as_str()=="Folder")).expect("marked test script");
    let guid=match script.properties.get(&"ScriptGuid".into()) {Some(rbx_types::Variant::String(g))=>g.clone(),_=>panic!("script GUID unavailable")};
    let svc=model.dom().descendants().find(|i|i.class.as_str()=="LiveScriptingService").expect("live scripting service");
    let service=model.server_id(svc.referent()).unwrap();
    session.send_event(service,"LiveScriptingService".into(),"LiveEditingClientConfiguration".into(),vec![PropValue::String("{\"version\":\"1\"}".into())]).unwrap();
    let source=match script.properties.get(&"Source".into()) {Some(rbx_types::Variant::String(s))=>s.clone(),_=>panic!("source unavailable")};
    let target=if source=="-- rustudio synchronization probe 🦊 é\nreturn 6\n" {"-- rustudio synchronization probe\nreturn 5\n"} else {"-- rustudio synchronization probe 🦊 é\nreturn 6\n"};
    session.send_event(service,"LiveScriptingService".into(),"SendAtomicOperation".into(),rustudio_net::scripts::replace(1,&guid,&source,target)).unwrap();
    println!("atomic whole-script replacement queued");
    let mut rejected_stale=false;
    let until=Instant::now()+Duration::from_secs(45);
    while Instant::now()<until {
        match session.events.recv_timeout(Duration::from_millis(100)) {
            Ok(LiveEvent::Batch(ops))=>for op in ops {
                if let RepOp::Event {id,name,args}=op {if id==service {
                    if name=="LiveScriptingService.SendAtomicUpdateResponse" {
                        println!("atomic response {args:?}");
                        if args==vec![PropValue::Int(1),PropValue::Enum(0)] {
                            session.send_event(service,"LiveScriptingService".into(),"SendAtomicOperation".into(),rustudio_net::scripts::replace(2,&guid,&source,"-- should never be applied\nreturn -1\n")).unwrap();
                            println!("stale-base commit queued for rejection test");
                        }
                        if args==vec![PropValue::Int(2),PropValue::Enum(2)] {rejected_stale=true;}
                    } else if name=="LiveScriptingService.BroadcastMessage" {println!("live scripting status event received");}
                }}
            },
            Ok(LiveEvent::Error(e) | LiveEvent::EditRejected(e)|LiveEvent::Disconnected(e)|LiveEvent::Notice(e))=>println!("live: {e}"),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)=>break,
            _=>{}
        }
    }
    assert!(rejected_stale,"server did not confirm stale-base rejection");
    println!("atomic replacement and stale-base rejection passed");
}
