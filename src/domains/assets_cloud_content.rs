//! Ephemeral DataModel-scoped image/mesh content. No publishing endpoint.
use super::*;
use std::sync::{atomic::{AtomicU64,Ordering},mpsc::{self,Receiver,TryRecvError}};
static NEXT:AtomicU64=AtomicU64::new(1);
#[derive(Default)]
struct ContentJobs{pending:HashMap<u64,Receiver<Result<Vec<u8>,String>>>,ids:Vec<String>}
impl Drop for ContentJobs{fn drop(&mut self){for id in &self.ids{crate::assets::unregister_generated(id);}}}
const UNSUPPORTED:&str="The input content type is not supported by AssetService::CreateDataModelContentAsync yet!";
enum Source{Image(crate::texture::Rgba),Mesh(Arc<Vec<u8>>)}
pub fn install(lua:&Lua)->mlua::Result<()>{
    lua.set_app_data(ContentJobs::default());
    let example=lua.create_userdata(crate::runtime::editable::LContent(rbx_dom_weak::types::Content::none()))?;
    let metatable=example.metatable()?;
    let tostring:mlua::Function=metatable.get("__tostring")?;
    metatable.set("__tostring",lua.create_function(move|_,value:Value|{
        if crate::runtime::editable::content_of(&value).and_then(|c|c.as_uri().map(str::to_owned)).is_some_and(|uri|uri.starts_with(crate::runtime::editable::OPAQUE_PREFIX)){
            return Ok("Content{SourceType=Opaque}".to_string())
        }
        tostring.call::<String>(value)
    })?)?;
    lua.globals().set("__ac_content_begin",lua.create_function(|lua,value:Value|{
        let result=(|| ->mlua::Result<u64>{
        let content=crate::runtime::editable::content_of(&value).ok_or_else(||rt_err("Unable to cast value to Content"))?;
        let r=content.as_object().ok_or_else(||rt_err(UNSUPPORTED))?;
        let class=world(lua)?.d().dom.get_by_ref(r).map(|i|i.class.to_string()).ok_or_else(||rt_err(UNSUPPORTED))?;
        let source=match class.as_str(){
            "EditableImage"=>{
                let uri=crate::runtime::editable::content_uri(r,None).ok_or_else(||rt_err(UNSUPPORTED))?;
                let (_,pixels)=crate::runtime::editable::image_pixels(&uri).ok_or_else(||rt_err(UNSUPPORTED))?;
                Source::Image(pixels)
            },
            "EditableMesh"=>{
                let snapshot=crate::runtime::editable::mesh_part_source(&content);
                if let Some(uri)=crate::runtime::editable::content_uri(r,None){lua.app_data_mut::<ContentJobs>().unwrap().ids.push(uri);}
                let (uri,_)=snapshot.ok_or_else(||rt_err("Error creating FileMeshData: Empty Result"))?.map_err(|_|rt_err("Error creating FileMeshData: Empty Result"))?;
                Source::Mesh(crate::assets::generated(&uri).ok_or_else(||rt_err("Error creating FileMeshData: Empty Result"))?)
            },
            _=>return Err(rt_err(UNSUPPORTED)),
        };
        let token=NEXT.fetch_add(1,Ordering::Relaxed);let(tx,rx)=mpsc::channel();
        lua.app_data_mut::<ContentJobs>().unwrap().pending.insert(token,rx);
        std::thread::spawn(move||{
            let result=match source{
                Source::Mesh(bytes)=>Ok((*bytes).clone()),
                Source::Image(image)=>(||{
                    let image=image::RgbaImage::from_raw(image.width,image.height,image.pixels).ok_or("Invalid image size")?;
                    let mut output=std::io::Cursor::new(Vec::new());
                    image::DynamicImage::ImageRgba8(image).write_to(&mut output,image::ImageFormat::Png).map_err(|e|e.to_string())?;
                    Ok(output.into_inner())
                })(),
            };let _=tx.send(result);
        });
        Ok(token)
        })();
        Ok(match result{Ok(token)=>(Some(token),None),Err(e)=>(None,Some(crate::runtime::describe_error(&e)))})
    })?)?;
    lua.globals().set("__ac_content_poll",lua.create_function(|lua,token:u64|{
        let result=(|| ->mlua::Result<(bool,Value)>{
        let mut jobs=lua.app_data_mut::<ContentJobs>().unwrap();
        let rx=jobs.pending.get(&token).ok_or_else(||rt_err("Unknown content request"))?;
        let result=match rx.try_recv(){Err(TryRecvError::Empty)=>return Ok((false,Value::Nil)),Err(_)=>Err("Content processor disconnected".into()),Ok(result)=>result};
        jobs.pending.remove(&token);
        let bytes=result.map_err(rt_err)?;
        let id=format!("{}datamodel/{token}",crate::runtime::editable::OPAQUE_PREFIX);
        crate::assets::register_generated(&id,bytes);jobs.ids.push(id.clone());drop(jobs);
        Ok((true,crate::runtime::editable::content_value(lua,&rbx_dom_weak::types::Content::from_uri(id))?))
        })();
        Ok(match result{Ok((done,value))=>(done,value,None),Err(e)=>(true,Value::Nil,Some(crate::runtime::describe_error(&e)))})
    })?)?;
    lua.load(r#"
__rs_lua_methods.CreateDataModelContentAsync=function(self,content,options)
    if content==nil then error('Argument 1 missing or nil',2) end
    if typeof(content)~='Content' and type(content)~='string' then error('Unable to cast value to Content',2) end
    if options~=nil and type(options)~='table' then error('Unable to cast value to Dictionary',2) end
    task.wait()
    local token,err=__ac_content_begin(content)
    if err then error(err,0) end
    while true do
        local done,value,err=__ac_content_poll(token)
        if err then error(err,0) end
        if done then return Enum.CreateContentResult.Success,value end
        task.wait()
    end
end
"#).set_name("=rustudio.assets_cloud.content").exec()
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn scoped_content_is_released(){
        let id=format!("{}scope-test/{}",crate::runtime::editable::OPAQUE_PREFIX,NEXT.fetch_add(1,Ordering::Relaxed));
        crate::assets::register_generated(&id,vec![1,2,3]);
        assert!(crate::assets::generated(&id).is_some());
        drop(ContentJobs{pending:HashMap::new(),ids:vec![id.clone()]});
        assert!(crate::assets::generated(&id).is_none());
    }
}
