use crate::avrcp::AvrcpAction;
use crate::error::{NocturnedError, Result};
use crate::websocket::WebSocketServer;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tracing::debug;

const POLL_INTERVAL: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize, Default)]
struct NowPlayingResponse {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    elapsed_ms: Option<u64>,
    #[serde(default)]
    artwork_base64: Option<String>,
    #[serde(default)]
    artwork_content_type: Option<String>,
}

pub struct ItunesBridge {
    base_url: String,
    client: reqwest::Client,
}

impl ItunesBridge {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("failed to build iTunes bridge HTTP client");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        }
    }

    fn action_path(action: AvrcpAction) -> &'static str {
        match action {
            AvrcpAction::Play => "play",
            AvrcpAction::Pause => "pause",
            AvrcpAction::Next => "next",
            AvrcpAction::Previous => "previous",
            AvrcpAction::VolumeUp => "volumeup",
            AvrcpAction::VolumeDown => "volumedown",
        }
    }

    pub async fn send_control(&self, action: AvrcpAction) -> Result<()> {
        let url = format!("{}/control/{}", self.base_url, Self::action_path(action));
        self.client
            .post(&url)
            .send()
            .await
            .map_err(|e| NocturnedError::General(anyhow::anyhow!(e)))?
            .error_for_status()
            .map_err(|e| NocturnedError::General(anyhow::anyhow!(e)))?;
        Ok(())
    }

    async fn fetch_now_playing(&self) -> Result<NowPlayingResponse> {
        let url = format!("{}/nowplaying", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| NocturnedError::General(anyhow::anyhow!(e)))?
            .error_for_status()
            .map_err(|e| NocturnedError::General(anyhow::anyhow!(e)))?;
        resp.json::<NowPlayingResponse>()
            .await
            .map_err(|e| NocturnedError::General(anyhow::anyhow!(e)))
    }
}

fn now_playing_json(np: &NowPlayingResponse) -> serde_json::Value {
    let mut media = serde_json::json!({});
    let mut has_media = false;
    if let Some(ref title) = np.title {
        media["MediaItemTitle"] = serde_json::json!(title);
        has_media = true;
    }
    if let Some(ref artist) = np.artist {
        media["MediaItemArtist"] = serde_json::json!(artist);
        has_media = true;
    }
    if let Some(ref album) = np.album {
        media["MediaItemAlbum"] = serde_json::json!(album);
        has_media = true;
    }
    if let Some(duration) = np.duration_ms {
        media["MediaItemDuration"] = serde_json::json!(duration);
        has_media = true;
    }

    let mut playback = serde_json::json!({
        "PlaybackAppName": "iTunes"
    });
    if let Some(ref status) = np.status {
        playback["PlaybackStatus"] = serde_json::json!(status);
    }
    if let Some(elapsed) = np.elapsed_ms {
        playback["PlaybackElapsedTime"] = serde_json::json!(elapsed);
    }

    let mut json = serde_json::json!({});
    if has_media {
        json["MediaItemAttributes"] = media;
    }
    json["PlaybackAttributes"] = playback;
    json
}

pub fn start_polling_task(ws_server: Arc<WebSocketServer>, bridge: Arc<ItunesBridge>) {
    tokio::spawn(async move {
        let mut last_track_key: Option<String> = None;
        loop {
            match bridge.fetch_now_playing().await {
                Ok(np) => {
                    ws_server
                        .broadcast_event(
                            "media.nowPlaying.update".to_string(),
                            now_playing_json(&np),
                        )
                        .await;

                    let track_key = format!(
                        "{}|{}|{}",
                        np.title.as_deref().unwrap_or(""),
                        np.artist.as_deref().unwrap_or(""),
                        np.album.as_deref().unwrap_or("")
                    );
                    if last_track_key.as_deref() != Some(track_key.as_str()) {
                        last_track_key = Some(track_key);
                        if let Some(ref data) = np.artwork_base64 {
                            ws_server
                                .broadcast_event(
                                    "media.nowPlaying.artwork".to_string(),
                                    serde_json::json!({
                                        "data": data,
                                        "contentType": np.artwork_content_type
                                            .clone()
                                            .unwrap_or_else(|| "image/jpeg".to_string())
                                    }),
                                )
                                .await;
                        }
                    }
                }
                Err(e) => {
                    debug!("iTunes bridge unreachable: {}", e);
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}
