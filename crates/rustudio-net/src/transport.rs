//! The wire transport to the RCC/CloudEdit server.
//!
//! Studio picks between two transports (docs/TEAMCREATE.md §4):
//!
//! - **RbxTransport**: QUIC (ngtcp2 + OpenSSL on Studio's side) to
//!   `NetStackPort`, directly or through a UDMUX relay with the RUPP token,
//!   plus Roblox's channel framing and a `RbxTransportSessionCrypto` layer.
//! - **RakNet**: Roblox's RakNet fork with the encrypted `RbxOpenRequest1/2`
//!   handshake and per-datagram AEAD (`SessionCrypto`).
//!
//! Both carry the same replicator item stream once connected (see
//! [`crate::replicator`]).
//!
//! [`connect_quic`] implements RbxTransport, including certificate pinning and
//! the Studio TLS capability extension. The live-session worker owns Roblox
//! channel framing, authentication, replication and heartbeats. RakNet remains
//! a separate research path; it is not the editor's live transport.

use crate::error::{NetError, Result};
use crate::joinscript::{JoinScript, TransportChoice};

/// Where and how to dial the server, distilled from a join script. Holds no
/// secrets directly — the tokens stay in the [`JoinScript`] and are read from
/// it at connect time, never copied into logs.
#[derive(Debug, Clone)]
pub struct DialPlan {
    pub choice: TransportChoice,
    /// Direct server endpoint (`MachineAddress` / `NetStackPort` or
    /// `ServerPort`).
    pub host: String,
    pub port: u16,
    /// UDMUX relay endpoints to try instead of a direct dial, if any.
    pub relays: Vec<(String, u16)>,
    pub direct_server_return: bool,
}

impl DialPlan {
    pub fn from_join(js: &JoinScript) -> DialPlan {
        let choice = js.transport_choice();
        let port = match choice {
            TransportChoice::RbxTransport => js.net_stack_port.unwrap_or(js.server_port),
            TransportChoice::RakNet => js.server_port,
        };
        DialPlan {
            choice,
            host: js.machine_address.clone(),
            port,
            relays: js
                .udmux_endpoints
                .iter()
                .map(|e| (e.address.clone(), e.port))
                .collect(),
            direct_server_return: js.direct_server_return,
        }
    }
}

/// Where the RbxTransport QUIC connection goes, from a join script
/// [verified: live 2026-11-05, Studio's log "UDMUX Address = …, Port = <NetStackPort>"]:
/// the UDMUX relay's address at `NetStackPort`, with a RUPP header naming the
/// RCC (`MachineAddress:NetStackPort`) and carrying the `NetStackTokenValue`.
pub struct QuicTarget {
    pub relay: std::net::SocketAddr,
    pub rcc: (std::net::Ipv4Addr, u16),
    token: [u8; 16],
    /// SHA-256 of the accepted server certificates (NetStackConfig.certHashes).
    cert_hashes: Vec<[u8; 32]>,
}

impl QuicTarget {
    pub fn from_join(js: &JoinScript) -> Result<QuicTarget> {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD;
        let port = js
            .net_stack_port
            .ok_or_else(|| NetError::JoinScript("no NetStackPort".into()))?;
        let rcc_ip: std::net::Ipv4Addr = js
            .machine_address
            .parse()
            .map_err(|_| NetError::JoinScript("MachineAddress is not IPv4".into()))?;
        let relay_ip: std::net::IpAddr = match js.udmux_endpoints.first() {
            Some(e) => e
                .address
                .parse()
                .map_err(|_| NetError::JoinScript("bad UDMUX address".into()))?,
            None => std::net::IpAddr::V4(rcc_ip),
        };
        let tok = b64
            .decode(js.net_stack_token_value.expose())
            .map_err(|_| NetError::JoinScript("NetStackTokenValue is not base64".into()))?;
        let token: [u8; 16] = tok
            .as_slice()
            .try_into()
            .map_err(|_| NetError::JoinScript(format!("NetStackTokenValue is {} bytes, not 16", tok.len())))?;
        let cert_hashes = js
            .net_stack_config
            .as_ref()
            .map(|c| {
                c.cert_hashes
                    .iter()
                    .filter_map(|h| b64.decode(h).ok()?.as_slice().try_into().ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok(QuicTarget {
            relay: std::net::SocketAddr::new(relay_ip, port),
            rcc: (rcc_ip, port),
            token,
            cert_hashes,
        })
    }
}

/// Accepts exactly the server certificates whose SHA-256 the join script
/// pinned (the RCC's certificate is self-signed), and checks the TLS 1.3
/// handshake signature against that certificate.
#[derive(Debug)]
struct PinnedCert {
    hashes: Vec<[u8; 32]>,
    algs: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl rustls::client::danger::ServerCertVerifier for PinnedCert {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let h = ring::digest::digest(&ring::digest::SHA256, end_entity.as_ref());
        if self.hashes.iter().any(|x| x.as_slice() == h.as_ref()) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("RCC certificate not pinned by the join script".into()))
        }
    }
    fn verify_tls12_signature(
        &self,
        _m: &[u8],
        _c: &rustls::pki_types::CertificateDer<'_>,
        _d: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 not used by QUIC".into()))
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &rustls::pki_types::CertificateDer<'_>,
        d: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// The RbxTransport ALPN [verified: capture].
pub const ALPN: &[u8] = b"rbx-rtcio";

/// Dial the RCC through the UDMUX relay: QUIC v1, ALPN `rbx-rtcio`,
/// TLS_AES_256_GCM_SHA384 preferred (Studio offers 0x1302 then 0x1303), the
/// certificate pinned by `certHashes`, RUPP framing underneath. Must run in a
/// tokio runtime. Returns the endpoint (keep it alive) and the connection.
pub async fn connect_quic(
    t: &QuicTarget,
) -> Result<(quinn::Endpoint, quinn::Connection, std::sync::Arc<crate::rupp::RuppSocket>)> {
    use std::sync::Arc;
    let provider = rustls::crypto::ring::default_provider();
    let algs = provider.signature_verification_algorithms;
    let mut provider = provider;
    provider.cipher_suites = vec![
        rustls::crypto::ring::cipher_suite::TLS13_AES_256_GCM_SHA384,
        rustls::crypto::ring::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
        rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256,
    ];
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NetError::Other(format!("tls: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCert {
            hashes: t.cert_hashes.clone(),
            algs,
        }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    // Studio 0.746 ClientHello private capability extension (capture j3).
    tls.rbx_transport_capabilities = Some(0x000d_d3e7_50f3_0892);
    let qc = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|e| NetError::Other(format!("quic tls: {e}")))?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(qc));
    let mut tc = quinn::TransportConfig::default();
    // The RUPP header (31 bytes) rides inside the path MTU, so stay at the
    // QUIC minimum and don't probe for more.
    tc.initial_mtu(1200);
    tc.min_mtu(1200);
    tc.mtu_discovery_config(None);
    tc.keep_alive_interval(Some(std::time::Duration::from_millis(1000)));
    tc.max_idle_timeout(Some(std::time::Duration::from_secs(30).try_into().unwrap()));
    tc.max_concurrent_uni_streams(256u32.into());
    tc.datagram_receive_buffer_size(Some(1 << 20));
    tc.stream_receive_window((64u32 << 20).into());
    tc.receive_window((128u32 << 20).into());
    cfg.transport_config(Arc::new(tc));

    let sock = crate::rupp::RuppSocket::bind(t.relay, t.rcc, t.token)
        .map_err(|e| NetError::Other(format!("udp bind: {e}")))?;
    let mut ep = quinn::Endpoint::new_with_abstract_socket(
        quinn::EndpointConfig::default(),
        None,
        sock.clone(),
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(|e| NetError::Other(format!("endpoint: {e}")))?;
    ep.set_default_client_config(cfg);
    let server_name = t.rcc.0.to_string();
    let conn = ep
        .connect(t.relay, &server_name)
        .map_err(|e| NetError::Other(format!("connect: {e}")))?
        .await
        .map_err(|e| NetError::Other(format!("QUIC handshake: {e}")))?;
    Ok((ep, conn, sock))
}

/// Build a QUIC client endpoint bound to an ephemeral local UDP port. This is
/// the RbxTransport foundation: the remaining pieces (ALPN, how the server
/// cert is checked against the ephemeral early public key, and the
/// `RbxTransportSessionCrypto` layer) get filled in from the capture. It makes
/// no network connection.
pub fn quic_client_endpoint() -> std::io::Result<quinn::Endpoint> {
    // Install the ring provider for rustls once (idempotent; ignore "already set").
    let _ = rustls::crypto::ring::default_provider().install_default();
    quinn::Endpoint::client("0.0.0.0:0".parse().expect("valid bind addr"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn capability_extension_in_actual_quic_client_hello() {
        fn extensions(capabilities: Option<u64>) -> Vec<Vec<u8>> {
            let mut cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_protocol_versions(&[&rustls::version::TLS13]).unwrap()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
            assert_eq!(cfg.rbx_transport_capabilities, None);
            cfg.rbx_transport_capabilities = capabilities;
            let mut conn = rustls::quic::ClientConnection::new(
                Arc::new(cfg), rustls::quic::Version::V1,
                "example.test".try_into().unwrap(), vec![],
            ).unwrap();
            let mut hello = Vec::new();
            conn.write_hs(&mut hello);
            assert_eq!(hello[0], 1); // ClientHello handshake, not a TLS record
            let mut pos = 4 + 2 + 32;
            pos += 1 + hello[pos] as usize; // legacy session id
            let suites = u16::from_be_bytes([hello[pos], hello[pos + 1]]) as usize;
            pos += 2 + suites;
            pos += 1 + hello[pos] as usize; // compression methods
            let len = u16::from_be_bytes([hello[pos], hello[pos + 1]]) as usize;
            pos += 2;
            let end = pos + len;
            let mut found = Vec::new();
            while pos < end {
                let ty = u16::from_be_bytes([hello[pos], hello[pos + 1]]);
                let len = u16::from_be_bytes([hello[pos + 2], hello[pos + 3]]) as usize;
                pos += 4;
                if ty == 0xff00 { found.push(hello[pos..pos + len].to_vec()); }
                pos += len;
            }
            assert_eq!(pos, end);
            found
        }
        assert!(extensions(None).is_empty());
        assert_eq!(extensions(Some(0x000d_d3e7_50f3_0892)),
            vec![vec![0x92, 0x08, 0xf3, 0x50, 0xe7, 0xd3, 0x0d, 0x00]]);
    }

    #[test]
    fn quic_stack_links_and_endpoint_builds() {
        // Proves quinn + rustls(ring) compile and a client endpoint binds,
        // with no network traffic.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let ep = quic_client_endpoint().expect("endpoint binds");
            assert!(ep.local_addr().unwrap().port() != 0);
        });
    }

    #[test]
    fn quic_target_from_join() {
        // base64 "0123456789abcdef" is 16 bytes; a fake cert hash.
        let js = JoinScript::parse(
            r#"{"MachineAddress":"10.1.2.3","ServerPort":100,"NetStackPort":64852,
                "NetStackTokenValue":"MDEyMzQ1Njc4OWFiY2RlZg==",
                "UdmuxEndpoints":[{"Address":"128.116.63.33","Port":64852}],
                "NetStackConfig":"{\"pubKey\":\"\",\"port\":64852,\"caps\":\"\",\"certHashes\":[\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\"]}"}"#,
        )
        .unwrap();
        let t = QuicTarget::from_join(&js).unwrap();
        assert_eq!(t.relay.to_string(), "128.116.63.33:64852");
        assert_eq!(t.rcc, ("10.1.2.3".parse().unwrap(), 64852));
        assert_eq!(t.token, *b"0123456789abcdef");
        assert_eq!(t.cert_hashes.len(), 1);
        assert_eq!(t.cert_hashes[0], [0u8; 32]);
    }

    #[test]
    fn dial_plan_prefers_netstack_port() {
        let js = JoinScript::parse(
            r#"{"MachineAddress":"10.0.0.1","ServerPort":1000,
                "NetStackPort":2000,"NetStackTokenValue":"bG9jYWxfdGVzdA=="}"#,
        )
        .unwrap();
        let p = DialPlan::from_join(&js);
        assert_eq!(p.choice, TransportChoice::RbxTransport);
        assert_eq!(p.port, 2000);
    }
}
