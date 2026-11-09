This is rustls 0.23.46, vendored from crates.io with its original licenses.

The RbxTransport patch adds an optional `ClientConfig::rbx_transport_capabilities`
field, defaulting to `None`. When set, the ClientHello carries private TLS
extension 0xff00 with the specified u64 encoded little-endian. Only rustudio's
RbxTransport client sets it; ordinary HTTPS clients omit it. Certificate
validation and TLS cryptography are unchanged.

Changed upstream files:
- src/client/client_conn.rs: optional configuration field
- src/client/builder.rs: default None
- src/client/hs.rs: ClientHello construction
- src/msgs/enums.rs: extension identifier
- src/msgs/handshake.rs: extension codec and owned conversion

The mask in rustudio-net is tied to the Studio 0.746 protocol template. Its
wire regression test constructs an actual QUIC ClientHello and checks both
the enabled extension bytes and default omission.
