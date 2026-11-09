//! SceneAnalysisService instance composition from the live DOM. Studio's
//! whitelist and ordered IsA categories:  / .
use super::*;
use std::collections::BTreeMap;
#[derive(Default)]
struct Memory { scripts:HashMap<Ref,u8>, active:HashMap<usize,u8>, instances:HashMap<Ref,Option<Ref>>, creations:u64, blocked:std::collections::HashSet<Ref> }
fn category(lua:&Lua,r:Ref)->u8{
    let Some(mut memory)=lua.app_data_mut::<Memory>()else{return 0};
    if let Some(&id)=memory.scripts.get(&r){return id}
    if memory.scripts.len()>=255{return 0}
    let id=(memory.scripts.len()+1)as u8;memory.scripts.insert(r,id);id
}
/// Attribute allocations to a script without changing its source or environment.
pub fn begin(lua:&Lua,r:Ref)->mlua::Result<u8>{
    let category=category(lua,r);let mut old=0;
    unsafe{lua.exec_raw::<()>((),|state|{
        if let Some(mut memory)=lua.app_data_mut::<Memory>(){old=memory.active.insert(state as usize,category).unwrap_or(0);}
        mlua::ffi::lua_setmemcat(state,category as i32);
    })}?;Ok(old)
}
pub fn restore(lua:&Lua,category:u8)->mlua::Result<()>{
    unsafe{lua.exec_raw::<()>((),|state|{
        if let Some(mut memory)=lua.app_data_mut::<Memory>(){memory.active.insert(state as usize,category);}
        mlua::ffi::lua_setmemcat(state,category as i32);
    })}
}
pub fn script_thread(lua:&Lua,thread:&mlua::Thread,r:Ref)->mlua::Result<()>{
    let category=category(lua,r);
    unsafe{lua.exec_raw::<()>(thread.clone(),|state|{
        let thread=mlua::ffi::lua_tothread(state,-1);
        if let Some(mut memory)=lua.app_data_mut::<Memory>(){memory.active.insert(thread as usize,category);}
        mlua::ffi::lua_setmemcat(thread,category as i32);
    })}
}
pub fn create_thread(lua:&Lua,f:mlua::Function)->mlua::Result<mlua::Thread>{
    let script=f.environment().and_then(|e|e.raw_get::<Value>("script").ok()).as_ref().and_then(opt_inst);
    let thread=lua.create_thread(f)?;
    if let Some(r)=script{script_thread(lua,&thread,r)?;}
    Ok(thread)
}
/// Class-level InternalTest access is checked before an Instance reaches Lua.
pub fn access_blocked(lua:&Lua,r:Ref)->bool{
    lua.try_app_data_ref::<Memory>().ok().flatten().is_some_and(|m|m.blocked.contains(&r))
}
pub fn scan_access(lua:&Lua,root:Ref)->mlua::Result<()>{
    let blocked={let w=world(lua)?;w.d().dom.descendants_of(root).filter(|i|i.class=="RealtimeMedia").map(|i|i.referent()).collect::<Vec<_>>()};
    if !blocked.is_empty(){if let Some(mut memory)=lua.app_data_mut::<Memory>(){memory.blocked.extend(blocked);}}
    Ok(())
}
/// Record only the allocation's creator; never retain an Instance userdata.
pub fn track_instance(lua:&Lua,r:Ref)->mlua::Result<()>{
    scan_access(lua,r)?;
    let mut category=0;
    unsafe{lua.exec_raw::<()>((),|state|{category=lua.app_data_ref::<Memory>().and_then(|m|m.active.get(&(state as usize)).copied()).unwrap_or(0);})}?;
    let cleanup=if let Some(mut memory)=lua.app_data_mut::<Memory>(){
        let creator=memory.scripts.iter().find_map(|(r,&id)|(id==category).then_some(*r));
        if creator.is_some(){memory.instances.entry(r).or_insert(creator);}
        memory.creations+=1;
        memory.creations%256==0
    }else{false};
    // Ref IDs are weak metadata; prune destroyed/collected instances without
    // holding an Instance userdata or letting diagnostics grow on every spawn.
    if cleanup{
        let refs=lua.app_data_ref::<Memory>().map(|m|m.instances.keys().copied().collect::<Vec<_>>()).unwrap_or_default();
        let dead={let w=world(lua)?;refs.into_iter().filter(|&r|!w.d().exists(r)).collect::<Vec<_>>()};
        if let Some(mut memory)=lua.app_data_mut::<Memory>(){for r in dead{memory.instances.remove(&r);}}
    }
    Ok(())
}
fn unparented(lua:&Lua)->mlua::Result<mlua::Table>{
    let origins=lua.app_data_ref::<Memory>().map(|m|m.instances.clone()).unwrap_or_default();
    let mut pending:Vec<(Ref,Option<Ref>)>=origins.iter().map(|(&r,&owner)|(r,owner)).collect();
    if let Some(c)=lua.app_data_ref::<Cache>(){pending.extend(c.slots.keys().map(|&r|(r,None)));}
    let w=world(lua)?;let d=w.d();let root=d.dom.root_ref();
    let mut seen=std::collections::HashSet::new();
    let mut groups:BTreeMap<String,BTreeMap<String,usize>>=BTreeMap::new();
    while let Some((r,inherited))=pending.pop(){
        if !seen.insert(r){continue}
        let Some(instance)=d.dom.get_by_ref(r)else{continue};
        let mut owner=origins.get(&r).copied().flatten().or(inherited);
        pending.extend(instance.children().iter().map(|&child|(child,owner)));
        let mut ancestor=r;let mut parented=false;
        while let Some(i)=d.dom.get_by_ref(ancestor){
            if owner.is_none(){owner=origins.get(&ancestor).copied().flatten();}
            if ancestor==root{parented=true;break}
            ancestor=i.parent();if ancestor.is_none(){break}
        }
        if parented{continue}
        let origin=owner.filter(|&r|d.exists(r)).map(|r|w.full_name(r)).unwrap_or_else(||"Unknown".into());
        *groups.entry(origin).or_default().entry(instance.class.to_string()).or_default()+=1;
    }
    drop(w);
    let children=lua.create_table()?;let mut total=0;
    for(i,(name,classes))in groups.into_iter().enumerate(){
        let size=classes.values().sum::<usize>();total+=size;let group=node(lua,&name,size)?;
        let leaves=lua.create_table()?;for(j,(class,count))in classes.into_iter().enumerate(){leaves.set(j+1,node(lua,&class,count)?)?;}
        group.set("Children",leaves)?;children.set(i+1,group)?;
    }
    let root=node(lua,"UnparentedInstances",total)?;root.set("Children",children)?;Ok(root)
}
fn script_memory(lua:&Lua)->mlua::Result<mlua::Table>{
    let scripts=lua.app_data_ref::<Memory>().map(|m|m.scripts.clone()).unwrap_or_default();
    let mut sizes=Vec::new();let mut total=0;
    unsafe{lua.exec_raw::<()>((),|state|{
        total=mlua::ffi::lua_totalbytes(state,-1);
        for(r,category)in scripts{sizes.push((r,mlua::ffi::lua_totalbytes(state,category as i32)));}
    })}?;
    let mut groups:BTreeMap<String,BTreeMap<&str,Vec<(String,usize)>>>=BTreeMap::new();
    {
        let w=world(lua)?;
        for(r,size)in sizes{
            if size==0{continue}
            let Some(instance)=w.d().dom.get_by_ref(r)else{continue};
            let path=w.full_name(r);let root=path.split('.').next().unwrap_or("").to_owned();
            let kind=if instance.class=="ModuleScript"{"ModuleScripts"}else{"Scripts"};
            groups.entry(root).or_default().entry(kind).or_default().push((path,size));
        }
    }
    let children=lua.create_table()?;
    for(i,(group,kinds))in groups.into_iter().enumerate(){
        let mut size=0;let kinds_table=lua.create_table()?;
        for(j,(kind,mut scripts))in kinds.into_iter().enumerate(){
            scripts.sort_by(|a,b|b.1.cmp(&a.1).then_with(||a.0.cmp(&b.0)));
            let subtotal=scripts.iter().map(|(_,n)|n).sum::<usize>();size+=subtotal;
            let leaves=lua.create_table()?;for(k,(name,size))in scripts.into_iter().enumerate(){leaves.set(k+1,node(lua,&name,size)?)?;}
            let kind=node(lua,kind,subtotal)?;kind.set("Children",leaves)?;kinds_table.set(j+1,kind)?;
        }
        let group=node(lua,&group,size)?;group.set("Children",kinds_table)?;children.set(i+1,group)?;
    }
    let root=node(lua,"ScriptMemory",total)?;root.set("Children",children)?;Ok(root)
}
const ROOTS:&[&str]=&["Lighting","MaterialService","Players","ReplicatedFirst","ReplicatedStorage","ServerScriptService","ServerStorage","SoundService","StarterGui","StarterPack","StarterPlayer","Teams","TextChatService","Workspace"];
const NAMES:&[&str]=&["3D Objects","Physics","UI","Lights","PostProcessing","Scripts","Audio","Animation","Values","Character","Textures","Meshes","Particles","Services & Storage","Misc","Unclassified"];
const CATEGORIES:&[(&str,usize)]=&[
    ("BasePart",0),
    ("Model",0),
    ("Camera",0),
    ("Attachment",1),
    ("Constraint",1),
    ("JointInstance",1),
    ("BodyMover",1),
    ("WeldConstraint",1),
    ("NoCollisionConstraint",1),
    ("TouchTransmitter",1),
    ("ControllerManager",1),
    ("ControllerBase",1),
    ("SensorBase",1),
    ("GuiBase",2),
    ("BasePlayerGui",2),
    ("UIBase",2),
    ("ProximityPrompt",2),
    ("ClickDetector",2),
    ("Dialog",2),
    ("DialogChoice",2),
    ("TextChatConfigurations",2),
    ("TextChannel",2),
    ("TextChatCommand",2),
    ("TextSource",2),
    ("Light",3),
    ("PostEffect",4),
    ("Highlight",4),
    ("Atmosphere",4),
    ("Sky",4),
    ("Clouds",4),
    ("LuaSourceContainer",5),
    ("BindableEvent",5),
    ("BindableFunction",5),
    ("RemoteEvent",5),
    ("RemoteFunction",5),
    ("Sound",6),
    ("SoundEffect",6),
    ("AudioPlayer",6),
    ("AudioListener",6),
    ("AudioDeviceOutput",6),
    ("Wire",6),
    ("SoundGroup",6),
    ("AudioEmitter",6),
    ("AudioFader",6),
    ("AudioCompressor",6),
    ("AudioEqualizer",6),
    ("Animator",7),
    ("AnimationController",7),
    ("Animation",7),
    ("KeyframeSequence",7),
    ("IKControl",7),
    ("ValueBase",8),
    ("CharacterAppearance",9),
    ("Accoutrement",9),
    ("Humanoid",9),
    ("HumanoidDescription",9),
    ("BaseWrap",9),
    ("FaceControls",9),
    ("AccessoryDescription",9),
    ("BodyPartDescription",9),
    ("FaceInstance",10),
    ("SurfaceAppearance",10),
    ("MaterialVariant",10),
    ("DataModelMesh",11),
    ("ParticleEmitter",12),
    ("Fire",12),
    ("Sparkles",12),
    ("Smoke",12),
    ("Explosion",12),
    ("Trail",12),
    ("Beam",12),
    ("Player",13),
    ("Folder",13),
    ("Configuration",13),
    ("Tool",14),
    ("Backpack",14),
    ("Team",14),
    ("ForceField",14),
    ("PathfindingModifier",14),
    ("PathfindingLink",14),
    ("PackageLink",14),
    ("HapticEffect",14),
    ("PlayerScripts",14),
    ("StarterPlayerScripts",14),
    ("StarterGear",14),
];
fn node(lua:&Lua,name:&str,size:usize)->mlua::Result<mlua::Table>{
    let t=lua.create_table()?;t.set("Name",name)?;t.set("Size",size)?;Ok(t)
}
fn composition(lua:&Lua)->mlua::Result<mlua::Table>{
    let w=world(lua)?;let d=w.d();
    let mut counts:Vec<BTreeMap<String,usize>>=(0..NAMES.len()).map(|_|BTreeMap::new()).collect();
    for &root in d.dom.root().children(){
        let Some(service)=d.dom.get_by_ref(root) else{continue};
        if !ROOTS.contains(&service.class.as_str()){continue}
        for instance in d.dom.descendants_of(root){
            if instance.referent()==root{continue}
            let class=instance.class.as_str();
            let category=CATEGORIES.iter().find(|(base,_)|crate::doc::is_a(class,base)).map(|(_,n)|*n).unwrap_or(15);
            *counts[category].entry(class.to_owned()).or_default()+=1;
        }
    }
    drop(w);
    let children=lua.create_table()?;let mut total=0;let mut index=1;
    for (i,classes) in counts.iter().enumerate(){
        let size=classes.values().sum::<usize>();if size==0{continue}
        let category=node(lua,NAMES[i],size)?;
        let leaves=lua.create_table()?;
        for (j,(class,count)) in classes.iter().enumerate(){leaves.set(j+1,node(lua,class,*count)?)?;}
        category.set("Children",leaves)?;children.set(index,category)?;index+=1;total+=size;
    }
    let root=node(lua,"InstanceComposition",total)?;root.set("Children",children)?;Ok(root)
}
pub fn install(lua:&Lua)->mlua::Result<()>{
    let blocked=lua.try_app_data_ref::<World>().ok().flatten().and_then(|w|w.doc.as_ref().map(|d|d.dom.descendants().filter(|i|i.class=="RealtimeMedia").map(|i|i.referent()).collect())).unwrap_or_default();
    lua.set_app_data(Memory{blocked,..Memory::default()});
    lua.globals().set("__ac_scene_mem_begin",lua.create_function(|lua,value:Value|begin(lua,opt_inst(&value).ok_or_else(||rt_err("Instance expected"))?))?)?;
    lua.globals().set("__ac_scene_mem_restore",lua.create_function(restore)?)?;
    lua.globals().set("__ac_unparented",lua.create_function(|lua,()|unparented(lua))?)?;
    lua.globals().set("__ac_script_memory",lua.create_function(|lua,()|script_memory(lua))?)?;
    lua.globals().set("__ac_instance_composition",lua.create_function(|lua,()|composition(lua))?)?;
    lua.load("__rs_lua_methods.GetInstanceCompositionAsync=function(self) task.wait(); return __ac_instance_composition() end; __rs_lua_methods.GetUnparentedInstancesAsync=function(self) task.wait(); return __ac_unparented() end; __rs_lua_methods.GetScriptMemoryAsync=function(self) task.wait(); return __ac_script_memory() end").set_name("=rustudio.assets_cloud.scene").exec()
}
