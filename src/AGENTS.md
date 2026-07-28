# NOCTURNED DAEMON — MODULE MAP

## OVERVIEW

Binary crate (`main.rs` entry). 17 modules, ~8.6K lines. Async tokio runtime orchestrating Bluetooth, iAP2, WebSocket, audio, and wake word subsystems.

## STRUCTURE

```
src/
├── main.rs                 # Entry point: tracing, config, image cache, WS server, audio, wakeword, BT daemon (184 lines)
├── bluetooth.rs            # RFCOMM listener, SDP registration, connection dispatch (1,738 lines)
├── bluetooth_agent.rs      # D-Bus Bluetooth agent for pairing (311 lines)
├── avrcp.rs                # Standard Bluetooth AVRCP transport commands via BlueZ org.bluez.MediaControl1
├── iap2_wrapper.rs         # Bridge to iap2-rs crate: config, events, EA session routing (873 lines)
├── websocket.rs            # WebSocket server on port 5000, UI broadcast (1,399 lines)
├── app/
│   ├── mod.rs              # AppCommunicationManager, AppMessage, session multiplexing (179 lines)
│   ├── msgpack.rs          # MsgPack RPC handler: chunking, CRC32, EA commands (1,604 lines)
│   └── websocket_handler.rs # WebSocket→iPhone command routing (297 lines)
├── audio.rs                # arecord capture, Opus encoding, broadcast channel (472 lines)
├── wakeword.rs             # ONNX-based wake word detector (457 lines)
├── mfi.rs                  # MFi chip interface: /dev/apple_mfi IOCTLs (266 lines)
├── mfi_impl.rs             # HardwareMfiProvider for iap2-rs auth trait (34 lines)
├── config.rs               # Config loading from /etc/nocturne/config.json (141 lines)
├── error.rs                # NocturnedError enum (thiserror) (46 lines)
├── ab.rs                   # A/B partition management via /dev/misc (198 lines)
├── brightness.rs           # Display brightness + ambient light sensor (299 lines)
└── image_cache.rs          # Disk-backed image cache for album art (95 lines)
```

## DATA FLOW

```
main.rs
  ├─ config::Config::load()
  ├─ brightness::init_brightness()
  ├─ image_cache::ImageCache::new()
  ├─ websocket::WebSocketServer::new(port=5000)
  ├─ audio::AudioCapture::new() → broadcast channel
  ├─ wakeword::WakeWordDetector::new() → event channel
  └─ bluetooth::BluetoothDaemon::new() → .run()
       └─ per-connection: iap2_wrapper::Iap2Connection::run()
            ├─ iap2-rs connect(stream, config)
            ├─ ConnectionEvent loop: Link → Auth → Identification → EA sessions
            ├─ AppCommunicationManager (app/mod.rs)
            │   ├─ MsgPackProtocolHandler (app/msgpack.rs) ← EA data
            │   └─ WebSocketProtocolHandler (app/websocket_handler.rs) ← WS data
            └─ NowPlaying state → WebSocket broadcast
```

## WHERE TO LOOK

| Task | File | Key Types/Functions |
|------|------|---------------------|
| Add Spotify command | `app/websocket_handler.rs` | `handle_message()` match arms |
| Add EA protocol handler | `app/mod.rs` | `AppProtocolHandlerEnum`, `register_handler()` |
| Modify chunking/CRC | `app/msgpack.rs` | `create_chunks()`, `parse_one_chunk_envelope()`, `process_inbound()` (per-session reassembly buffer), CHUNK_SIZE=2000 |
| iAP2 config (EA protocols, NowPlaying) | `iap2_wrapper.rs` | `Iap2Config` construction |
| MFi auth | `mfi.rs` + `mfi_impl.rs` | `MfiChip`, `HardwareMfiProvider` |
| Bluetooth SDP/advertising | `bluetooth.rs` | `register_sdp_record()`, `set_advertising()` |
| Audio pipeline | `audio.rs` | `AudioCapture`, `AudioCommand::Start/Stop` |
| Wake word | `wakeword.rs` | `WakeWordDetector`, ONNX model loading from `/etc/nocturne/models` |
| WebSocket events to UI | `websocket.rs` | `broadcast_event()`, `WebSocketServer` |
| OTA updates | `app/msgpack.rs` | `download_ota_chunks_task()`, MD5 verification |
| Display sleep | `brightness.rs`, `websocket.rs` | `sleep_display()`, `wake_display()`, `device.display.*`; transient backlight sleep, separate from UI lock, never persisted brightness |
| `media.control.*` routing | `app/hid_mapping.rs` (shared mapping), `avrcp.rs` (BlueZ AVRCP sender), `iap2_wrapper.rs` (WebSocket source + channel receiver), `app/msgpack.rs` (phone EA source) | Both sources resolve via `method_to_avrcp_action`. WebSocket path calls `avrcp::send` directly; Phone EA path forwards through `hid_tx` channel, drained in `run_iap2_connection`'s select loop, which also calls `avrcp::send` — see MEDIA CONTROL / AVRCP below |
| `notification.show` (UI alerts) | `app/msgpack.rs` (explicit log branch) | Phone-emitted events with `{id, title, body, category, daysUntilExpiry, timestamp}`. Forwarded to WebSocket UI clients via the existing `broadcast_event` fallthrough; the explicit branch only adds logging. Consumed by `nocturne-ui/src/components/common/notifications/NotificationBridge.jsx`. |
| Phone reconnect / session repair | `bluetooth.rs` (`connect_to_device`), `iap2_wrapper.rs` (RequestAppLaunch in main loop) | Reconnect **policy lives in the UI** (`nocturne-ui/src/hooks/useNocturned.js` watchdog) — phones never reconnect on their own, so the UI dials `bluetooth.device.connect` through the daemon, including when BlueZ reports connected but no `bluetooth.connection` session is live. Daemon side: (1) `connect_to_device` is idempotent — returns `connected` immediately if an iAP2/SPP session already exists, so UI dials are always safe; (2) on a **cold start only** — an iAP2 link is up but no EA session has *ever* arrived on this link within 5s (iOS app not running) — the daemon sends `RequestAppLaunch` for `com.usenocturne.nocturne` (retry every 15s, max 5). Gated by the per-`run_iap2_connection` `ea_session_ever_established` latch: once the app opens an EA session even once, the daemon will **never** force-foreground it again on that link (even if the user backgrounds the app and the EA session drops). This is intentional — foregrounding is "open once per drive, then leave the user alone"; mid-drive re-foregrounds over Maps were the bug. The latch resets only when the iAP2 link fully cycles (a fresh `run_iap2_connection`). Does **not** touch `app_ready_received` (the `daemon.ready` resend gate). The live foreground path is actually the UI's `device.launchApp`-on-`app.ready` — see `nocturne-ui/src/hooks/useNocturned.js` (`appLaunchRequested` once-per-drive gate); this daemon latch is defense-in-depth. |
| macOS connector reconnect | `bluetooth.rs` (`connect_to_device`, `probe_macos_connector`) | Computer-class paired devices are treated as macOS connector targets before the iAP2 channel `1` trial: the daemon opens a short RFCOMM probe to the Mac's Bluetooth-Incoming-Port listener on channel `3`, then waits for the Mac app to dial back to the Car Thing SPP/RPC server on channel `2`. Do not move this into a Mac-side polling loop; the Mac connector must only dial channel `2` in response to the inbound probe. Android wake remains the fallback for non-computer targets. |

## CONVENTIONS

- All IO through tokio async — never block the runtime
- `tracing::{info, debug, warn, error}` for structured logging everywhere
- Error propagation: `Result<T>` with `NocturnedError` via `thiserror`
- Channel patterns: `mpsc::unbounded_channel` for command/event routing, `broadcast` for audio
- Module-level constants for hardware paths (not configurable — device-specific)

## MEDIA CONTROL / AVRCP

`media.control.*` RPCs arrive from TWO sources with TWO different emission paths, but both ultimately send standard Bluetooth AVRCP transport commands via BlueZ — NOT Apple's proprietary iAP2 HID component. Playback is controlled directly at the OS/Bluetooth level (same mechanism as steering-wheel buttons), so it works against any AVRCP target (Apple Music, Spotify, etc.) with no MFi auth, no EA session, and no internet connection required on the Car Thing.

`src/avrcp.rs` owns the transport: `AvrcpAction` enum (`Play`, `Pause`, `Next`, `Previous`, `VolumeUp`, `VolumeDown`) and `send(address: bluer::Address, action: AvrcpAction) -> Result<()>`, which does a blocking D-Bus call to `org.bluez.MediaControl1` on the connected device's object path (found via `ObjectManager::get_managed_objects()`, same lookup pattern as `bluetooth::disconnect_device`/`unpair_device`).

Both sources share the canonical method-string-to-`AvrcpAction` mapping helper in `src/app/hid_mapping.rs` (`method_to_avrcp_action(&str) -> Option<avrcp::AvrcpAction>`):

- **UI WebSocket** (Car Thing browser → daemon): handled inline in `iap2_wrapper::handle_websocket_message_new` at `src/iap2_wrapper.rs:~810-836`. Resolves via `crate::app::hid_mapping::method_to_avrcp_action`, then emits DIRECTLY via `crate::avrcp::send(device_address, cmd).await` — bypassing the channel.

- **Phone EA** (iOS nocturne-app → daemon via MessagePack RPC): handled in `MsgPackProtocolHandler::handle_msgpack_message` at `src/app/msgpack.rs:~525`. The handler is configured with a clone of `hid_tx: UnboundedSender<AvrcpAction>` via `MsgPackProtocolHandler::set_hid_tx(...)` (declared at `src/app/msgpack.rs:~209`, wired at `src/iap2_wrapper.rs:~288`). When `method.starts_with("media.control.")`, the handler resolves via `method_to_avrcp_action` and forwards through `hid_tx.send(cmd)`; the receiver `hid_rx` (drained in `run_iap2_connection`'s select loop) calls `crate::avrcp::send(device_address, cmd).await`.

**Architectural dichotomy (intentional)**: Both paths share the same mapping helper but emit through DIFFERENT call sites (direct vs. channel). The `hid_tx`/`hid_rx` naming is legacy from the pre-AVRCP HID implementation — kept to minimize churn, but it now carries `AvrcpAction`, not `iap2_rs::HidCommand`.

Supported methods (6 total, both sources): `play`, `pause`, `next`, `previous`/`prev`, `volumeUp`, `volumeDown`. `shuffle`, `repeat`, `playPause`/`togglePlayPause` are NOT supported — `org.bluez.MediaControl1` has no toggle/shuffle/repeat primitives, only discrete transport commands.

`org.bluez.MediaControl1` is deprecated in newer BlueZ in favor of `org.bluez.MediaPlayer1` + `org.bluez.MediaTransport1` (which also allows absolute, not just relative, volume). If a target BlueZ build doesn't expose `MediaControl1`, `avrcp.rs` is the only file that needs to change.

## ANTI-PATTERNS

- **Don't add `lib.rs`**: This is intentionally a binary crate; shared types live in module files
- **Don't refactor hardcoded paths**: `/etc/nocturne/`, `/dev/apple_mfi`, `/dev/misc` are Car Thing filesystem constants
- **Don't block tokio**: Audio/wakeword use `tokio::spawn` with internal buffering
- **Chunk envelope format is fixed**: iOS app expects exact 36-char UUID message IDs, CRC32 checksums, 2000-byte chunks — changing breaks the wire protocol
- **`media.control.*` routing shares ONE canonical mapping helper**: `src/app/hid_mapping.rs` is the single source of truth for method-string→`AvrcpAction`. Don't duplicate or hardcode `media.control.*` strings in callers. The WebSocket path (inline at `src/iap2_wrapper.rs:~810-836`) emits directly via `crate::avrcp::send`; the Phone EA path (`src/app/msgpack.rs:~525`) emits via the `hid_tx` channel through `MsgPackProtocolHandler`. These are intentional separate paths. Don't migrate the WebSocket path to `hid_tx` without a follow-up plan.
- **Don't route media control back through `iap2_rs::HidCommand`/`send_hid_command`**: that's Apple's proprietary iAP2 HID component and requires MFi auth + an active EA session. Playback control now goes through `src/avrcp.rs` (standard Bluetooth AVRCP via BlueZ D-Bus), which works independent of the iAP2/EA session state.
