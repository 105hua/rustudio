//! Read-only Studio asset endpoints, recovered from the shipped Studio binary.
//! All requests carry the user's login session; no publishing routes exist here.
use crate::{Error, Result, Session};
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum Read {
    CaptureStatus { token: String, server: bool },
    Bundle(u64),
    Latest(u64),
    AudioSearch(Value),
    AudioMetadata(Vec<u64>),
    Package(u64),
    Free { decals: bool, keyword: String, page: i32 },
    Places { universe: u64, cursor: String },
}
impl Read {
    pub fn url(&self) -> String {
        match self {
            Self::CaptureStatus {token,server}=>format!("https://apis.roblox.com/content-captures-api/v1/check-upload-status{}?token={}",if *server{"-rcc"}else{""},component(token)),
            Self::AudioSearch(_) => "https://apis.roblox.com/toolbox-service/v2/assets:search".into(),
            Self::AudioMetadata(ids) => format!("https://apis.roblox.com/asset-metadata-retrieval-service/v1/audio/metadata?{}",ids.iter().map(|id|format!("assetIds={id}")).collect::<Vec<_>>().join("&")),
            Self::Latest(id) => format!("https://develop.roblox.com/v2/assets/{id}/versions"),
            Self::Bundle(id) => format!("https://catalog.roblox.com/v1/bundles/{id}/details"),
            Self::Package(id) => format!("https://inventory.roblox.com/v1/packages/{id}/assets"),
            Self::Free {decals,keyword,page} => format!("https://apis.roblox.com/toolbox-service/v1/legacy-insert-service/{}?keyword={}&pageNumber={page}",if *decals {"Decal"} else {"Model"}, component(keyword)),
            Self::Places {universe,cursor} => format!("https://develop.roblox.com/v1/universes/{universe}/places?sortOrder=Asc&limit=50{}",if cursor.is_empty(){String::new()}else{format!("&cursor={}",component(cursor))}),
        }
    }
    pub fn fetch(&self, session: &Session) -> Result<Value> {
        self.fetch_url(session, &self.url())
    }
    fn fetch_url(&self, session: &Session, url: &str) -> Result<Value> {
        if matches!(self,Self::CaptureStatus{..}) {
            let (status,body)=session.get(url).map_err(|_|Error::new("CheckUploadCaptureStatusAsync: GET Request Network Error"))?;
            return capture_status(status,&body);
        }
        let (status, body) = match self {
            Self::AudioSearch(body) => session.post_csrf(url,"application/json",&serde_json::to_vec(body)?)?,
            _=>session.get(url)?,
        };
        if !(200..300).contains(&status) { return Err(Error::new(if matches!(self,Self::CaptureStatus{..}) {"CheckUploadCaptureStatusAsync: Unexpected status code".into()} else {format!("HTTP {status}")})); }
        serde_json::from_str(&body).map_err(|_| Error::new("Server Response in not valid Json"))
    }
}

// 202 is pending; 200 decodes result.assetId,
// result.isApproved, result.operationError into UploadCaptureResult + asset ID.
fn capture_status(status:u16,body:&str)->Result<Value>{
    let result=if status==202 {(6,0)} else {
        if status!=200{return Err(Error::new("CheckUploadCaptureStatusAsync: Unexpected status code"))}
        let json:Value=serde_json::from_str(body).map_err(|_|Error::new("CheckUploadCaptureStatusAsync: GET Response Parse Error"))?;
        if !json.is_object(){return Err(Error::new("CheckUploadCaptureStatusAsync: GET Response Parse Error"))}
        let r=json.get("result").filter(|v|v.is_object()).ok_or_else(||Error::new("CheckUploadCaptureStatusAsync: Missing result object"))?;
        let asset=r.get("assetId").and_then(Value::as_u64).unwrap_or(0);
        if r.get("operationError").and_then(Value::as_str).is_some_and(|s|!s.is_empty()){(7,0)}
        else if !r.get("isApproved").and_then(Value::as_bool).unwrap_or(false){(2,asset)}
        else if asset==0{(6,0)}else{(0,asset)}
    };
    Ok(serde_json::json!({"resultCode":result.0,"assetId":result.1}))
}

fn component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() { if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {out.push(b as char)} else {out.push_str(&format!("%{b:02X}"))} }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cookie, Profile};
    use crate::mock::{MockServer, MockReply};
    use std::sync::Arc;
    #[test]
    fn capture_status_native_results(){
        for (status,body,code,id) in [
            (202,"",6,0),(200,r#"{"result":{"assetId":7,"isApproved":true}}"#,0,7),
            (200,r#"{"result":{"assetId":7,"isApproved":false}}"#,2,7),
            (200,r#"{"result":{"assetId":0,"isApproved":true}}"#,6,0),
            (200,r#"{"result":{"assetId":7,"isApproved":true,"operationError":"failed"}}"#,7,0),
        ]{let v=capture_status(status,body).unwrap();assert_eq!(v["resultCode"],code);assert_eq!(v["assetId"],id);}
        for(status,body,message)in [(400,"","Unexpected status code"),(200,"invalid","GET Response Parse Error"),(200,"{}","Missing result object")]{assert_eq!(capture_status(status,body).unwrap_err().to_string(),format!("CheckUploadCaptureStatusAsync: {message}"));}
    }
    #[test]
    fn asset_routes_and_session() {
        assert_eq!(Read::CaptureStatus{token:"a &b".into(),server:false}.url(), "https://apis.roblox.com/content-captures-api/v1/check-upload-status?token=a%20%26b");
        assert_eq!(Read::CaptureStatus{token:"a".into(),server:true}.url(), "https://apis.roblox.com/content-captures-api/v1/check-upload-status-rcc?token=a");
        assert_eq!(Read::Bundle(7).url(), "https://catalog.roblox.com/v1/bundles/7/details");
        assert_eq!(Read::Package(7).url(), "https://inventory.roblox.com/v1/packages/7/assets");
        assert_eq!(Read::Free{decals:false,keyword:"a &b".into(),page:2}.url(), "https://apis.roblox.com/toolbox-service/v1/legacy-insert-service/Model?keyword=a%20%26b&pageNumber=2");
        let server=MockServer::start(Arc::new(|r| {
            assert_eq!(r.method,"GET");
            assert_eq!(r.header("cookie"),Some(".ROBLOSECURITY=mock-session"));
            MockReply::json(200,"{\"assetIds\":[7,8]}")
        })).unwrap();
        let session=Session::unchecked(Cookie::new("mock-session").unwrap(),Profile{id:1,name:"Mock".into(),display_name:"Mock".into()});
        assert_eq!(Read::Package(7).fetch_url(&session,&server.base).unwrap()["assetIds"],serde_json::json!([7,8]));
    }
    #[test]
    #[ignore = "explicit signed-in read-only capture status smoke"]
    fn live_capture_status(){
        let c=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        for server in [false,true]{
            match (Read::CaptureStatus{token:"invalid".into(),server}).fetch(&c.session){
                Ok(v)=>{println!("capture status server={server}: {v}");assert_eq!(v,serde_json::json!({"resultCode":6,"assetId":0}));}
                Err(e)=>{println!("capture status server={server}: {e}");assert_eq!(e.to_string(),"CheckUploadCaptureStatusAsync: Unexpected status code");}
            }
        }
    }
    #[test]
    #[ignore = "explicit signed-in read-only smoke"]
    fn live_assets_cloud_reads() {
        let c=crate::Client::open(crate::AccountConfig::load().unwrap()).unwrap_or_else(|e|panic!("{}",e.describe()));
        for read in [Read::Bundle(1),Read::Package(27112438),Read::Latest(1818),Read::AudioMetadata(vec![1843529600]),Read::AudioSearch(serde_json::json!({"searchCategoryType":"Audio","searchView":"Core","query":"piano","sortCategory":"Relevance","sortDirection":"Descending","maxPageSize":10,"audioSearchFilters":{"audioTypes":["Music"]}})),Read::Free{decals:false,keyword:"tree".into(),page:1}] {
            match read.fetch(&c.session) { Ok(v)=>{
                println!("{}: response keys {:?}",read.url(),v.as_object().map(|o|o.keys().collect::<Vec<_>>()));
                if let Some(row)=v.pointer("/audioMetadataResults/0") {
                    println!("metadata row keys {:?}",row.as_object().map(|o|o.keys().collect::<Vec<_>>()));
                    println!("metadata value keys {:?}",row.get("audioMetadata").and_then(|v|v.as_object()).map(|o|o.keys().collect::<Vec<_>>()));
                }
                if let Some(row)=v.pointer("/creatorStoreAssets/0") {
                    println!("audio row keys {:?}",row.as_object().map(|o|o.keys().collect::<Vec<_>>()));
                    if let Some(id)=row.pointer("/asset/id").and_then(|v|v.as_u64().or_else(||v.as_str().and_then(|s|s.parse().ok()))) {
                        let details=Read::AudioMetadata(vec![id]).fetch(&c.session).unwrap();
                        println!("store audio metadata row keys {:?}",details.pointer("/audioMetadataResults/0").and_then(|v|v.as_object()).map(|o|o.keys().collect::<Vec<_>>()));
                        println!("store audio metadata keys {:?}",details.pointer("/audioMetadataResults/0/audioMetadata").and_then(|v|v.as_object()).map(|o|o.keys().collect::<Vec<_>>()));
                    }

                    for key in ["asset","creator","audioDetails"] {println!("{key} keys {:?}",row.get(key).and_then(|v|v.as_object()).map(|o|o.keys().collect::<Vec<_>>()));}
                }
            }, Err(e)=>println!("{}: {}",read.url(),e) }
        }
    }
}
