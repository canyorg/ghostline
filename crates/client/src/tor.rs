//! Embedded Tor (Arti — the Tor Project's Rust implementation).
//!
//! No Tor Browser and no system tor service needed: the app carries its own Tor client.
//!
//! - **Circuits:** every connection goes through Tor's usual 3 hops; to an onion relay
//!   that is 6 hops with no exit node.
//! - **Per-room isolation:** each room gets its own isolated client, so two rooms never
//!   share a circuit (see [`EmbeddedTor::isolated`]).
//! - **Rotation:** the HTTP layer swaps to fresh circuits every [`ROTATE_EVERY`]
//!   (see `net.rs`).
//! - **State:** like Tor Browser, Arti keeps its guard choice and a directory cache on
//!   disk. Long-lived guards *protect* you (fewer chances to pick a malicious guard).
//!   Set `GHOSTLINE_AMNESIC=1` to use a throw-away directory instead (deleted on exit;
//!   slower start, new guards every run).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use arti_client::config::TorClientConfigBuilder;
use arti_client::{TorClient, TorClientConfig};
use futures::StreamExt;
use tor_rtcompat::PreferredRuntime;

/// How often the HTTP layer moves to brand-new circuits.
pub const ROTATE_EVERY: Duration = Duration::from_secs(10 * 60);

/// Progress callback: fraction 0..1 and a short status text.
pub type Progress = Arc<dyn Fn(f32, String) + Send + Sync>;

/// A bootstrapped, in-process Tor client.
pub struct EmbeddedTor {
    client: Arc<TorClient<PreferredRuntime>>,
    amnesic_dir: Option<PathBuf>,
}

impl std::fmt::Debug for EmbeddedTor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EmbeddedTor")
    }
}

fn dirs_for_state() -> Result<(PathBuf, PathBuf, Option<PathBuf>)> {
    if std::env::var("GHOSTLINE_AMNESIC").is_ok_and(|v| v == "1") {
        let mut tag = [0u8; 8];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut tag);
        let base = std::env::temp_dir().join(format!(
            "ghostline-{}",
            tag.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        return Ok((base.join("state"), base.join("cache"), Some(base)));
    }
    let base = dirs::data_local_dir()
        .context("cannot find a data directory for Tor state")?
        .join("ghostline")
        .join("arti");
    Ok((base.join("state"), base.join("cache"), None))
}

impl EmbeddedTor {
    /// Start Tor and wait until it is ready. Must be called from inside a tokio runtime.
    pub async fn start(progress: Progress) -> Result<Arc<Self>> {
        let (state, cache, amnesic_dir) = dirs_for_state()?;
        let config: TorClientConfig = TorClientConfigBuilder::from_directories(&state, &cache)
            .build()
            .context("invalid Tor configuration")?;

        progress(0.0, "starting Tor".into());
        let client = TorClient::builder()
            .config(config)
            .create_unbootstrapped()
            .context("could not start Tor")?;

        let mut events = client.bootstrap_events();
        let p = progress.clone();
        let watcher = tokio::spawn(async move {
            while let Some(st) = events.next().await {
                p(st.as_frac(), st.to_string());
                if st.ready_for_traffic() {
                    break;
                }
            }
        });

        client.bootstrap().await.context(
            "could not connect to the Tor network (is the internet up? is Tor blocked here?)",
        )?;
        watcher.abort();
        progress(1.0, "Tor ready".into());
        Ok(Arc::new(EmbeddedTor {
            client,
            amnesic_dir,
        }))
    }

    /// A client whose circuits are isolated from every other isolated client.
    pub fn isolated(&self) -> Arc<TorClient<PreferredRuntime>> {
        self.client.isolated_client()
    }
}

impl Drop for EmbeddedTor {
    fn drop(&mut self) {
        if let Some(dir) = &self.amnesic_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
