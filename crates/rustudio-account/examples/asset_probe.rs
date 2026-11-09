//! Probe asset delivery for one asset under several request shapes (read-only).
//! usage: asset_probe ASSET PLACE
fn main() {
    let mut a = std::env::args().skip(1);
    let id: u64 = a.next().unwrap().parse().unwrap();
    let place: u64 = a.next().unwrap().parse().unwrap();
    let c = rustudio_account::Client::open(rustudio_account::AccountConfig::load().unwrap()).unwrap();
    let s = &c.session;
    for (ctx, url) in [
        (0, format!("https://assetdelivery.roblox.com/v1/asset/?id={id}")),
        (place, format!("https://assetdelivery.roblox.com/v1/asset/?id={id}")),
        (place, format!("https://assetdelivery.roblox.com/v1/asset/?id={id}&serverplaceid={place}&scriptinsert=1&clientinsert=0")),
        (0, format!("https://assetdelivery.roblox.com/v1/asset/?id={id}&serverplaceid={place}&scriptinsert=1&clientinsert=0")),
    ] {
        rustudio_account::session::set_place_context(ctx);
        match s.fetch_bytes(&url, true) {
            Ok((st, b)) => println!("ctx={ctx} {url}\n  -> {st} {} bytes {:?}", b.len(), String::from_utf8_lossy(&b[..b.len().min(160)])),
            Err(e) => println!("ctx={ctx} {url}\n  -> err {e}"),
        }
    }
    rustudio_account::session::set_place_context(place);
    println!("v2: {:?}", s.download_asset(id, None).map(|b| b.len()).map_err(|e| e.to_string()));
}
