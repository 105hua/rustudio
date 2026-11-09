//! The join script returned by `POST gamejoin.roblox.com/v1/team-create`.
//!
//! The matchmaker reply is a `GameJoinResponse`; its `joinScript` (or
//! `joinScriptUrl`) carries a `JoinInformation` object. Studio's native
//! parsers read the fields listed in `docs/TEAMCREATE.md` §3.4. We model the set
//! rustudio needs to bring up a transport, and accept both the PascalCase
//! form (`TeamCreateJoinPayload`, as RCC emits) and the camelCase
//! `JoinInformation` form, via serde aliases.
//!
//! Token/ticket/key fields are held as [`Redacted`] so they can't be printed.

use crate::Redacted;
use serde::Deserialize;

/// One `{Address, Port}` entry (ServerConnections / UdmuxEndpoints).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ServerEndpoint {
    #[serde(alias = "Address", alias = "address")]
    pub address: String,
    #[serde(alias = "Port", alias = "port")]
    pub port: u16,
}

/// The transport rustudio should bring up for this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportChoice {
    /// RbxTransport (QUIC) to `NetStackPort` with the NetStack token. Chosen
    /// when the join script carries a NetStack port, a NetStack token and the
    /// server's ephemeral public key (the CloudEdit/Team Create default,
    /// `FFlagUseRbxTransportClient3`).
    RbxTransport,
    /// Roblox RakNet with the encrypted RbxOpenRequest1/2 handshake, to
    /// `MachineAddress:ServerPort`. The fallback when there is no usable
    /// NetStack endpoint.
    RakNet,
}

/// Raw deserialization target. Not public; [`JoinScript`] is the clean view.
#[derive(Debug, Deserialize)]
struct Raw {
    #[serde(default, alias = "GameId", alias = "gameId")]
    game_id: Option<String>,
    #[serde(default, alias = "UniverseId", alias = "universeId")]
    universe_id: Option<u64>,
    #[serde(default, alias = "PlaceId", alias = "placeId")]
    place_id: Option<u64>,
    #[serde(default, alias = "DataCenterId", alias = "dataCenterId")]
    data_center_id: Option<u64>,
    #[serde(default, alias = "MachineAddress", alias = "machineAddress")]
    machine_address: Option<String>,
    #[serde(default, alias = "ServerPort", alias = "serverPort")]
    server_port: Option<u16>,
    #[serde(default, alias = "ServerConnections", alias = "serverConnections")]
    server_connections: Option<Vec<ServerEndpoint>>,
    #[serde(default, alias = "UdmuxEndpoints", alias = "udmuxEndpoints")]
    udmux_endpoints: Option<Vec<ServerEndpoint>>,
    #[serde(default, alias = "DirectServerReturn", alias = "directServerReturn")]
    direct_server_return: Option<bool>,
    #[serde(default, alias = "NetStackPort", alias = "netStackPort")]
    net_stack_port: Option<u16>,
    #[serde(default, alias = "SessionId", alias = "sessionId")]
    session_id: Option<String>,
    #[serde(default, alias = "UserName", alias = "userName")]
    user_name: Option<String>,
    #[serde(default, alias = "RccVersion", alias = "rccVersion")]
    rcc_version: Option<String>,

    // Secret-bearing fields.
    #[serde(default, alias = "TokenValue", alias = "tokenValue")]
    token_value: Option<String>,
    #[serde(default, alias = "NetStackTokenValue", alias = "netStackTokenValue")]
    net_stack_token_value: Option<String>,
    #[serde(default, alias = "ClientTicket", alias = "clientTicket")]
    client_ticket: Option<String>,
    #[serde(
        default,
        alias = "EphemeralEarlyPubKey",
        alias = "ephemeralEarlyPubKey",
        alias = "RbxTransportEphemeralEarlyPublicKey"
    )]
    ephemeral_early_pub_key: Option<String>,
    #[serde(default, alias = "ClientPublicKeyData", alias = "clientPublicKeyData")]
    client_public_key_data: Option<String>,
    /// URL-encoded JSON `{pubKey, port, caps, certHashes[]}` (RbxTransport).
    #[serde(default, alias = "NetStackConfig", alias = "netStackConfig")]
    net_stack_config: Option<String>,
    #[serde(default, alias = "UserId", alias = "userId")]
    user_id: Option<u64>,
}

/// `NetStackConfig` from the join script [verified: live 2026-11-05]: the
/// RbxTransport server's static public key, its port, a 12-byte capability
/// blob and the SHA-256 hashes of the QUIC server certificate(s), which pin
/// the TLS certificate (self-signed, as for WebTransport).
#[derive(Debug, Clone, Default)]
pub struct NetStackConfig {
    pub pub_key: Redacted,
    pub port: Option<u16>,
    pub caps: Redacted,
    /// base64 SHA-256 of each acceptable server certificate (DER).
    pub cert_hashes: Vec<String>,
}

/// Percent-decoding for the URL-encoded join-script fields.
pub fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl NetStackConfig {
    fn parse(raw: &str) -> Option<NetStackConfig> {
        let text = if raw.trim_start().starts_with('{') {
            raw.to_string()
        } else {
            url_decode(raw)
        };
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        };
        Some(NetStackConfig {
            pub_key: Redacted::new(s("pubKey")),
            port: v.get("port").and_then(|x| x.as_u64()).map(|p| p as u16),
            caps: Redacted::new(s("caps")),
            cert_hashes: v
                .get("certHashes")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|h| h.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// The parsed join script, with secrets redacted for display.
#[derive(Debug, Clone)]
pub struct JoinScript {
    pub game_id: Option<String>,
    pub universe_id: Option<u64>,
    pub place_id: Option<u64>,
    pub data_center_id: Option<u64>,
    /// Primary RCC endpoint (`MachineAddress:ServerPort`).
    pub machine_address: String,
    pub server_port: u16,
    pub server_connections: Vec<ServerEndpoint>,
    pub udmux_endpoints: Vec<ServerEndpoint>,
    pub direct_server_return: bool,
    pub net_stack_port: Option<u16>,
    pub session_id: Option<String>,
    pub user_name: Option<String>,
    pub rcc_version: Option<String>,

    /// RUPP/relay token (`TokenValue`). Empty for a local test server.
    pub token_value: Redacted,
    /// RbxTransport (QUIC) token (`NetStackTokenValue`). For a local Studio
    /// server this is `bG9jYWxfdGVzdA==` (base64 "local_test").
    pub net_stack_token_value: Redacted,
    /// Semicolon-separated RbxTransport SessionCrypto parameters. Studio
    /// Base64-decodes fields 3/4 into its `0xa8` hello and parses the final
    /// field as the suite selector.
    pub client_ticket: Redacted,
    /// URL-encoded Base64 server key, separate from the `ClientTicket` bundle.
    /// Absent on a local test server.
    pub ephemeral_early_pub_key: Redacted,
    pub client_public_key_data: Redacted,
    pub net_stack_config: Option<NetStackConfig>,
    pub user_id: Option<u64>,
}

impl JoinScript {
    /// Parse a `JoinInformation` object (the inner `joinScript`).
    pub fn parse(json: &str) -> Result<JoinScript, String> {
        let raw: Raw = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let machine_address = raw
            .machine_address
            .clone()
            .or_else(|| {
                raw.server_connections
                    .as_ref()?
                    .first()
                    .map(|c| c.address.clone())
            })
            .ok_or_else(|| "no MachineAddress or ServerConnections".to_string())?;
        let server_port = raw
            .server_port
            .or_else(|| raw.server_connections.as_ref()?.first().map(|c| c.port))
            .ok_or_else(|| "no ServerPort".to_string())?;
        Ok(JoinScript {
            game_id: raw.game_id,
            universe_id: raw.universe_id,
            place_id: raw.place_id,
            data_center_id: raw.data_center_id,
            machine_address,
            server_port,
            server_connections: raw.server_connections.unwrap_or_default(),
            udmux_endpoints: raw.udmux_endpoints.unwrap_or_default(),
            direct_server_return: raw.direct_server_return.unwrap_or(false),
            net_stack_port: raw.net_stack_port,
            session_id: raw.session_id,
            user_name: raw.user_name,
            rcc_version: raw.rcc_version,
            token_value: Redacted::new(raw.token_value.unwrap_or_default()),
            net_stack_token_value: Redacted::new(raw.net_stack_token_value.unwrap_or_default()),
            client_ticket: Redacted::new(raw.client_ticket.unwrap_or_default()),
            ephemeral_early_pub_key: Redacted::new(url_decode(
                &raw.ephemeral_early_pub_key.unwrap_or_default(),
            )),
            client_public_key_data: Redacted::new(raw.client_public_key_data.unwrap_or_default()),
            net_stack_config: raw
                .net_stack_config
                .as_deref()
                .and_then(NetStackConfig::parse),
            user_id: raw.user_id,
        })
    }

    /// Which transport to try first. RbxTransport when there is a NetStack
    /// port and token; RakNet otherwise. Mirrors Studio's `Transport
    /// selection` (docs/TEAMCREATE.md §4).
    pub fn transport_choice(&self) -> TransportChoice {
        let has_netstack = self.net_stack_port.is_some() && !self.net_stack_token_value.is_empty();
        if has_netstack {
            TransportChoice::RbxTransport
        } else {
            TransportChoice::RakNet
        }
    }

    /// True when this looks like a local Studio test server (empty tokens /
    /// no relays / `local_test` NetStack token). Used to iterate offline.
    pub fn is_local_test(&self) -> bool {
        self.token_value.is_empty()
            && self.client_ticket.is_empty()
            && self.udmux_endpoints.is_empty()
            && (self.net_stack_token_value.expose() == "bG9jYWxfdGVzdA=="
                || self.net_stack_token_value.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The shape a local Studio server hands its client: a local test
    // server, no Roblox traffic. No real secrets here ("local_test" is
    // Studio's own fixed local token).
    const LOCAL_TEST: &str = r#"{
        "MachineAddress": "10.77.0.2",
        "ServerPort": 53640,
        "ServerConnections": [{"Address": "10.77.0.2", "Port": 53640}],
        "UdmuxEndpoints": [],
        "DirectServerReturn": true,
        "NetStackPort": 53640,
        "NetStackTokenValue": "bG9jYWxfdGVzdA==",
        "TokenValue": "",
        "ClientTicket": "",
        "PlaceId": 0,
        "UniverseId": 0
    }"#;

    #[test]
    fn parses_local_test_and_picks_rbxtransport() {
        let js = JoinScript::parse(LOCAL_TEST).unwrap();
        assert_eq!(js.machine_address, "10.77.0.2");
        assert_eq!(js.server_port, 53640);
        assert_eq!(js.net_stack_port, Some(53640));
        assert_eq!(js.server_connections.len(), 1);
        assert!(js.direct_server_return);
        assert!(js.is_local_test());
        assert_eq!(js.transport_choice(), TransportChoice::RbxTransport);
    }

    #[test]
    fn secrets_never_display() {
        let js = JoinScript::parse(LOCAL_TEST).unwrap();
        // NetStackTokenValue is held; it must not render as its value.
        assert_eq!(format!("{}", js.net_stack_token_value), "<redacted>");
        assert_eq!(format!("{:?}", js.net_stack_token_value), "<redacted>");
    }

    #[test]
    fn camelcase_joininformation_aliases() {
        let camel = r#"{
            "machineAddress": "128.116.0.9",
            "serverPort": 49152,
            "udmuxEndpoints": [{"address": "128.116.0.9", "port": 49152}],
            "netStackPort": 49152,
            "netStackTokenValue": "QUJD",
            "tokenValue": "WFla",
            "clientTicket": "VGtU",
            "ephemeralEarlyPubKey": "a2V5",
            "placeId": 100000000000004,
            "universeId": 10000000003
        }"#;
        let js = JoinScript::parse(camel).unwrap();
        assert_eq!(js.place_id, Some(100000000000004));
        assert_eq!(js.universe_id, Some(10000000003));
        assert_eq!(js.udmux_endpoints.len(), 1);
        assert!(!js.is_local_test()); // has real tokens
        assert_eq!(js.transport_choice(), TransportChoice::RbxTransport);
    }

    #[test]
    fn raknet_when_no_netstack() {
        let js_json = r#"{"MachineAddress":"1.2.3.4","ServerPort":1000}"#;
        let js = JoinScript::parse(js_json).unwrap();
        assert_eq!(js.transport_choice(), TransportChoice::RakNet);
    }
}
