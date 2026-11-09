//! InsertService consumes a Roblox model, not an upload's source image/mesh.
//! The upload ledger deliberately maps IDs to originals; only model or FBX
//! originals may bypass the delivered Roblox model (see regression tests).

pub fn model_bytes(bytes: &[u8]) -> &[u8] {
    let mut bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    while bytes.first().is_some_and(u8::is_ascii_whitespace) { bytes = &bytes[1..]; }
    if bytes.starts_with(b"<?xml") {
        if let Some(end) = bytes.windows(2).position(|w| w == b"?>") {
            bytes = &bytes[end + 2..];
            while bytes.first().is_some_and(u8::is_ascii_whitespace) { bytes = &bytes[1..]; }
        }
    }
    bytes
}

fn is_model_source(bytes: &[u8]) -> bool {
    model_bytes(bytes).starts_with(b"<roblox") || crate::fbx::is_fbx(bytes)
}

pub fn select_model_source(
    local: Option<Vec<u8>>,
    delivered: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    if let Some(bytes) = local.filter(|b| is_model_source(b)) { return Ok(bytes); }
    delivered()
}

pub fn asset_bytes(id: u64) -> Result<Vec<u8>, String> {
    let local = crate::assets::local_asset_path(id).and_then(|p| std::fs::read(p).ok());
    select_model_source(local, || crate::assets::remote_cached(id).map(|b| (*b).clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;
    // Actual session-delivered model for Place A Factory's decal 80366567625947.
    // SHA256 33da5e0f17e7945ee6942404aae24c0163b2377f91251ceb65219f3bd7d1cbe4.
    const FACTORY_DECAL: &[u8] = br#"<roblox xmlns:xmime="http://www.w3.org/2005/05/xmlmime" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:noNamespaceSchemaLocation="http://www.roblox.com/roblox.xsd" version="4">
  <External>null</External>
  <External>nil</External>
  <Item class="Decal" referent="RBX0">
    <Properties>
      <token name="Face">5</token>
      <string name="Name">Decal</string>
      <float name="Shiny">20</float>
      <float name="Specular">0</float>
      <Content name="Texture">
        <url>http://www.roblox.com/asset/?id=103327827611569</url>
      </Content>
      <bool name="archivable">true</bool>
    </Properties>
  </Item>
</roblox>"#;

    #[test]
    fn real_factory_decal_ignores_png_upload_source() {
        // Actual first 32 bytes of its ledger source PNG. The former loader
        // passed these image bytes to the model decoder instead of the model.
        let png_source = [0x89,0x50,0x4e,0x47,0x0d,0x0a,0x1a,0x0a,0,0,0,0x0d,0x49,0x48,0x44,0x52,0,0,4,0,0,0,2,0,8,2,0,0,0,0x93,0xaf,0x89];
        assert!(Document::decode(&png_source).unwrap_err().contains("Invalid file header"));
        let delivered = std::cell::Cell::new(false);
        let bytes = select_model_source(Some(png_source.to_vec()), || {
            delivered.set(true);
            Ok(FACTORY_DECAL.to_vec())
        }).unwrap();
        assert!(delivered.get());
        let dom = Document::decode(model_bytes(&bytes)).unwrap();
        let decal = dom.get_by_ref(dom.root().children()[0]).unwrap();
        assert_eq!(decal.class.as_str(), "Decal");
        assert_eq!(decal.properties[&rbx_dom_weak::ustr("TextureContent")], rbx_dom_weak::types::Variant::Content(rbx_dom_weak::types::Content::from_uri("http://www.roblox.com/asset/?id=103327827611569")));
    }

    #[test]
    fn real_model_xml_and_binary_still_bypass_download() {
        let mut declared = b"\xef\xbb\xbf  <?xml version=\"1.0\"?>\n".to_vec();
        declared.extend_from_slice(FACTORY_DECAL);
        let selected = select_model_source(Some(declared), || panic!("model must not download")).unwrap();
        let dom = Document::decode(model_bytes(&selected)).unwrap();
        let mut binary = Vec::new();
        rbx_binary::to_writer(&mut binary, &dom, dom.root().children()).unwrap();
        let selected = select_model_source(Some(binary), || panic!("binary model must not download")).unwrap();
        assert_eq!(Document::decode(model_bytes(&selected)).unwrap().root().children().len(), 1);
    }
}
