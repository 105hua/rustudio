//! Capture rustudio's own rendered play viewport. No external gallery/upload calls.
use super::*;
use std::sync::{LazyLock, atomic::{AtomicU64,Ordering}, mpsc::{self,Receiver,Sender,TryRecvError}};
#[derive(Default)]
struct FrameQueue { pending:Vec<Sender<Result<Vec<u8>,String>>>, waiting:bool }
static FRAMES:LazyLock<Mutex<FrameQueue>>=LazyLock::new(||Mutex::new(FrameQueue::default()));
static NEXT:AtomicU64=AtomicU64::new(1);
#[derive(Default)]
struct Captures { pending:HashMap<u64,(Ref,Receiver<Result<Vec<u8>,String>>)>, objects:HashMap<Ref,Info>, prompts:Vec<mlua::Function> }
struct Info {time:i64,id:String,place:i64,universe:i64}
pub const LUA_METHODS:&[(&str,&str)]=&[("CaptureScreenshot","CaptureService"),("TakeScreenshotCaptureAsync","CaptureService"),("StartVideoCaptureAsync","CaptureService"),("StopVideoCapture","CaptureService"),("ReadCapturesFromGalleryAsync","CaptureService"),("PromptSaveCapturesToGallery","CaptureService"),("PromptShareCapture","CaptureService")];
pub const EVENTS:&[(&str,&str)]=&[];
fn capture_type(lua:&Lua)->mlua::Result<Value>{Ok(Value::UserData(lua.create_userdata(LEnumItem::from_name("CaptureType","Screenshot").unwrap())?))}
/// Called by the editor after drawing the viewport; captures only its own GPU frame.
pub fn frame_tick(ctx:&egui::Context, rect:egui::Rect) {
    let mut queue=FRAMES.lock().unwrap_or_else(|e|e.into_inner());
    if queue.pending.is_empty(){queue.waiting=false;return}
    if queue.waiting {
        let shot=ctx.input(|i|i.raw.events.iter().find_map(|e|match e {egui::Event::Screenshot{image,..}=>Some(image.clone()),_=>None}));
        if let Some(shot)=shot {
            let ppp=ctx.pixels_per_point();
            let x0=(rect.min.x*ppp).max(0.) as u32;
            let y0=(rect.min.y*ppp).max(0.) as u32;
            let x1=((rect.max.x*ppp).max(0.) as u32).min(shot.size[0] as u32);
            let y1=((rect.max.y*ppp).max(0.) as u32).min(shot.size[1] as u32);
            let result=(||{
                if x1<=x0 || y1<=y0{return Err("Capture viewport is unavailable".into())}
                let mut bytes=Vec::with_capacity(((x1-x0)*(y1-y0)*4)as usize);
                for y in y0..y1 {for x in x0..x1 {let p=shot.pixels[y as usize*shot.size[0]+x as usize];bytes.extend_from_slice(&[p.r(),p.g(),p.b(),255]);}}
                let image=image::RgbaImage::from_raw(x1-x0,y1-y0,bytes).ok_or("Invalid capture size")?;
                let mut out=std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(image).write_to(&mut out,image::ImageFormat::Png).map_err(|e|e.to_string())?;
                Ok(out.into_inner())
            })();
            for tx in queue.pending.drain(..){let _=tx.send(result.clone());}
            queue.waiting=false;
        }
    } else {
        queue.waiting=true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        ctx.request_repaint();
    }
}
pub fn install(lua:&Lua)->mlua::Result<()> {
    lua.set_app_data(Captures::default());
    // Content.fromObject accepts a Capture too (Studio gallery probe). Keep
    // editable-object conversion delegated to its existing implementation.
    let content:mlua::Table=lua.globals().get("Content")?;
    let from_object:mlua::Function=content.get("fromObject")?;
    content.set("fromObject",lua.create_function(move|lua,value:Value|{
        if let Some(r)=opt_inst(&value){
            if lua.app_data_ref::<Captures>().unwrap().objects.contains_key(&r){
                return crate::runtime::editable::content_value(lua,&rbx_dom_weak::types::Content::from_referent(r));
            }
        }
        from_object.call::<Value>(value)
    })?)?;
    lua.globals().set("__ac_capture_begin",lua.create_function(|lua,provider:Value| {
        let r=opt_inst(&provider).ok_or_else(||rt_err("Instance expected"))?;
        let token=NEXT.fetch_add(1,Ordering::Relaxed);
        let(tx,rx)=mpsc::channel();
        FRAMES.lock().unwrap_or_else(|e|e.into_inner()).pending.push(tx);
        lua.app_data_mut::<Captures>().unwrap().pending.insert(token,(r,rx));
        Ok(token)
    })?)?;
    lua.globals().set("__ac_capture_poll",lua.create_function(|lua,token:u64| {
        let mut captures=lua.app_data_mut::<Captures>().unwrap();
        let(provider,rx)=captures.pending.get(&token).ok_or_else(||rt_err("Unknown capture"))?;
        let _provider=*provider;
        let result=match rx.try_recv(){Err(TryRecvError::Empty)=>return Ok((false,Value::Nil,Value::Nil,None)),Err(_)=>Err("Capture renderer disconnected".into()),Ok(v)=>v};
        captures.pending.remove(&token);drop(captures);
        match result {
            Ok(bytes)=>{
                let id=format!("rbxtemp://rustudio/capture/{token}.png");
                crate::assets::register_generated(&id,bytes);
                let r=crate::runtime::ext::create_any(lua,"ScreenshotCapture")?;
                let (place,universe)={let w=world(lua)?;let d=w.d();let root=d.dom.root_ref();let get=|key|match d.prop(root,key){Some(Variant::Int64(n))=>n,_=>0};(get("PlaceId"),get("GameId"))};
                lua.app_data_mut::<Captures>().unwrap().objects.insert(r,Info{time:LDateTime::now().millis,id:id.clone(),place,universe});
                Ok((true,Value::String(lua.create_string(id)?),inst_value(lua,r)?,None))
            },
            Err(e)=>Ok((true,Value::Nil,Value::Nil,Some(e))),
        }
    })?)?;
    lua.globals().set("__ac_capture_known",lua.create_function(|lua,(value,allow_object):(Value,bool)|{
        let captures=lua.app_data_ref::<Captures>().unwrap();
        if allow_object {if let Some(r)=opt_inst(&value){return Ok(captures.objects.contains_key(&r))}}
        let uri=match &value {
            Value::String(s)=>Some(s.to_string_lossy()),
            Value::UserData(u)=>u.borrow::<crate::runtime::editable::LContent>().ok().and_then(|c|c.0.as_uri().map(str::to_owned)),
            _=>None,
        };
        Ok(uri.is_some_and(|id|captures.objects.values().any(|info|info.id==id)))
    })?)?;
    lua.globals().set("__ac_capture_prompt",lua.create_function(|lua,callbacks:mlua::Table|{
        let mut captures=lua.app_data_mut::<Captures>().unwrap();
        for callback in callbacks.sequence_values::<mlua::Function>(){captures.prompts.push(callback?);}
        Ok(())
    })?)?;
    lua.load(r#"
local M=__rs_lua_methods
local function capture(self,callback,object)
    if type(callback)~='function' then error('Unable to cast value to function',3) end
    local token=__ac_capture_begin(self)
    task.spawn(function()
        while true do
            local done,id,instance,err=__ac_capture_poll(token)
            if done then
                if object then callback(err and Enum.ScreenshotCaptureResult.OtherError or Enum.ScreenshotCaptureResult.Success,instance)
                elseif not err then callback(id) else warn(err) end
                return
            end
            task.wait()
        end
    end)
end
-- The Studio Play frontend does not present these native gallery prompts.
-- Native calls keep callbacks pending; no gallery save/share event is emitted.
M.PromptSaveCapturesToGallery=function(self,captures,callback)
    if captures==nil then error('Argument 1 missing or nil',2) end
    if type(captures)~='table' then error('Unable to cast value to Array',2) end
    if callback==nil then error('Argument 2 missing or nil',2) end
    if type(callback)~='function' then error('Unable to cast value to function',2) end
    if __ac_is_server() then error('PromptSaveCapturesToGallery can only be called on the client',0) end
    if #captures==0 then error('PromptSaveCapturesToGallery: Invalid contents',0) end
    for _,value in captures do
        if not __ac_capture_known(value,true) then error('PromptSaveCapturesToGallery: Invalid content',0) end
    end
    __ac_capture_prompt({callback})
end
M.PromptShareCapture=function(self,content,launchData,accepted,denied)
    if content==nil then error('Argument 1 missing or nil',2) end
    if launchData==nil then error('Argument 2 missing or nil',2) end
    if accepted==nil then error('Argument 3 missing or nil',2) end
    if denied==nil then error('Argument 4 missing or nil',2) end
    if typeof(content)~='Content' and type(content)~='string' then error('Unable to cast value to Content',2) end
    if type(accepted)~='function' or type(denied)~='function' then error('Unable to cast value to function',2) end
    if __ac_is_server() then error('PromptShareCapture can only be called on the client',0) end
    if not __ac_capture_known(content,false) then error('PromptShareCapture: Invalid capture Content: '..tostring(content),0) end
    __ac_capture_prompt({accepted,denied})
end
-- Desktop Studio has no video capture device and no granted gallery
-- permission. These paths do not issue gallery/upload requests or callbacks.
M.StartVideoCaptureAsync=function(self,callback,params)
    if type(callback)~='function' then error('Unable to cast value to function',2) end
    if params~=nil and type(params)~='table' then error('Unable to cast value to Dictionary',2) end
    task.wait()
    return Enum.VideoCaptureStartedResult.NoDeviceSupport
end
M.StopVideoCapture=function(self) end
M.ReadCapturesFromGalleryAsync=function(self,filters,allExperiences)
    if filters~=nil and type(filters)~='table' then error('Unable to cast value to Array',2) end
    task.wait()
    return Enum.ReadCapturesFromGalleryResult.NeedPermission
end
M.CaptureScreenshot=function(self,callback) capture(self,callback,false) end
M.TakeScreenshotCaptureAsync=function(self,callback,params) capture(self,callback,true) end
"#).set_name("=rustudio.assets_cloud.capture").exec()
}
pub fn prop_get(lua:&Lua,r:Ref,class:&str,key:&str)->mlua::Result<Option<Value>> {
    if !crate::doc::is_a(class,"Capture"){return Ok(None)}
    let captures=lua.app_data_ref::<Captures>().unwrap();
    let Some(info)=captures.objects.get(&r) else{return Ok(None)};
    Ok(match key {
        "CaptureTime"=>Some(Value::UserData(lua.create_userdata(LDateTime{millis:info.time})?)),
        "CaptureType"=>Some(capture_type(lua)?),
        "LocalId"=>Some(Value::String(lua.create_string("")?)),
        "SourcePlaceId"=>Some(Value::Integer(info.place)),
        "SourceUniverseId"=>Some(Value::Integer(info.universe)),
        _=>None,
    })
}
