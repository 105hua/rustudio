//! Studio CSM Test-channel SignalR JSON transport. Credentials never leave Session.
use crate::{Session, Error, Result};
use super::Context;
use tungstenite::{client::IntoClientRequest, Message, WebSocket, stream::MaybeTlsStream};
use std::net::TcpStream;
use serde_json::{Value, json};
pub struct Connection { socket: WebSocket<MaybeTlsStream<TcpStream>>, next: u64 }
impl Connection {
    pub fn connect(session: &Session, context: Context) -> Result<Self> {
        let mut request=format!("wss://csm.roblox.com/v1/router/?id=&universeId={}",context.universe).into_client_request().map_err(|_|Error::new("MessagingService: Invalid connection context"))?;
        for (key,value) in [("Cookie",session.cookie().header()),("X-Roblox-ChannelType","Test".into()),("Roblox-Place-Id",context.place.to_string()),("Roblox-Universe-Id",context.universe.to_string()),("Roblox-Game-Id",String::new()),("Roblox-Job-Signature",String::new()),("accessKey",String::new()),("User-Agent","RobloxStudio/WinInet RobloxApp/0.744.0.7441077 (GlobalDist; RobloxDirectDownload)".into())] {
            request.headers_mut().insert(tungstenite::http::HeaderName::from_bytes(key.as_bytes()).unwrap(),value.parse().map_err(|_|Error::new("MessagingService: Invalid context header"))?);
        }
        let (socket,_) = tungstenite::connect(request).map_err(|e| match e { tungstenite::Error::Http(r)=>Error::new(format!("MessagingService: CSM handshake refused (HTTP {})",r.status().as_u16())),_=>Error::new("MessagingService: CSM connection failed") })?;
        let mut this=Self{socket,next:0};
        this.send(json!({"protocol":"json","version":1}))?;
        Ok(this)
    }
    pub fn timeout(&mut self, duration: std::time::Duration) {
        match self.socket.get_mut() {
            MaybeTlsStream::Plain(s)=>{let _=s.set_read_timeout(Some(duration));},
            MaybeTlsStream::Rustls(s)=>{let _=s.sock.set_read_timeout(Some(duration));},
            _=>{}
        }
    }
    fn send(&mut self, value: Value)->Result<()> {
        self.socket.send(Message::Text(format!("{value}\u{1e}").into())).map_err(|_|Error::new("MessagingService: CSM send failed"))
    }
    pub fn invoke(&mut self,target:&str,args:Value)->Result<String> {
        let id=self.next.to_string();self.next+=1;
        self.send(json!({"type":1,"target":target,"arguments":args,"invocationId":id}))?;Ok(id)
    }
    pub fn keepalive(&mut self)->Result<()> {self.send(json!({"type":6}))}
    pub fn read(&mut self)->Result<Vec<Value>> {
        match self.socket.read() {
            Ok(Message::Text(s))=>s.split('\u{1e}').filter(|s|!s.is_empty()).map(|s|serde_json::from_str(s).map_err(|_|Error::new("MessagingService: Invalid CSM response"))).collect(),
            Ok(Message::Ping(_))=>{let _=self.socket.flush();Ok(vec![])},
            Ok(Message::Close(_))=>Err(Error::new("MessagingService: CSM connection closed")),
            Ok(_)=>Ok(vec![]),
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>Ok(vec![]),
            Err(_)=>Err(Error::new("MessagingService: CSM receive failed"))
        }
    }
}
#[cfg(test)]
mod tests {
 use super::*;
 #[test] #[ignore="authorized scratch universe only; real CSM publish"]
 fn scratch_csm_roundtrip(){
  let (cookie,_)=crate::session::load_cookie(true).unwrap().unwrap();let session=Session::validate(cookie).unwrap();
  let mut c=Connection::connect(&session,Context{place:10000000000008,universe:10000000007,players:0}).unwrap();c.timeout(std::time::Duration::from_secs(3));
  println!("CSM handshake {:?}",c.read().unwrap());
  c.invoke("Subscribe",json!(["rustudio_scratch_transport",0])).unwrap();
  println!("CSM subscribe {:?}",c.read().unwrap());
  c.invoke("Publish",json!(["rustudio_scratch_transport",json!({"Sent":1794128000.0,"Data":"scratch-only","TraceId":0,"TracingEnabled":false}).to_string(),0])).unwrap();
  for _ in 0..4 {println!("CSM result {:?}",c.read().unwrap());}
 }
}
