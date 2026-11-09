These four fragments are Studio 0.746 build-specific protocol data.

Source binary SHA-256:
7005e4f922fa44034884e84ad03fef943672c2b0b2eca46df5cc154331e46297

- studio_0746_sync_head.bin (1602 bytes): packet 0x90 header, Luau range,
  synchronized flag names; stops before the player JSON length.
- studio_0746_sync_tail.bin (20 bytes): five build-specific u32 values.
- studio_0746_ticket_middle.bin (87 bytes): version/build/platform metadata
  between the client ticket and the SessionId JSON length.
- studio_0746_ticket_tail.bin (4 bytes): packet 0x8a trailer.

Extraction excludes user IDs, both JSON objects, tickets, cookies, keys,
shared-string cache hashes, and all other captured messages. Per-session
values are built from the authenticated matchmaker response in live.rs.
The TLS private capability mask in transport.rs must match this protocol build.
