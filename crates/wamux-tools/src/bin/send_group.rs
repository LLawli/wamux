//! Validation helper: reconnect the paired account from PERSISTED state (no
//! re-pairing), list participating groups, and send a message to the group whose
//! subject starts with the given needle (default "Equação A").
//!
//! Usage: send_group [needle] [text]
//! Env:   DATABASE_URL (defaults to the local docker postgres)

use std::sync::Arc;
use std::time::Duration;

use tracing_subscriber::EnvFilter;

use wamux::domain::{groups, jid_parse, messaging};
use wamux::proto::v1 as pb;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::storage;

const EXTERNAL_REF: &str = "pair-socket";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn,wamux=info,whatsapp_rust=warn")),
        )
        .init();

    let needle = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Equação A".to_string());
    let text = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "wamux: teste de envio em grupo ✅".to_string());

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wamux:wamux@localhost:5433/wamux".to_string());
    let engine = Arc::new(storage::postgres::PgStorage::open(&database_url, 8).await?);
    let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(256)));
    registry.load_existing().await?;

    let account_ref = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(EXTERNAL_REF.to_string())),
    };
    let handle = registry
        .resolve(Some(&account_ref))
        .map_err(|e| anyhow::anyhow!("account '{EXTERNAL_REF}' not found: {e}"))?;

    println!(
        "Reconnecting account {} (device_id={}) from persisted state ...",
        handle.uuid, handle.device_id
    );
    registry.connect(&handle, None, true).await?;

    let client = loop {
        if let Some(client) = handle.client().await {
            break client;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    // Wait until logged in (reconnect uses the persisted Signal state — no QR).
    let mut connected = false;
    for _ in 0..120 {
        if handle.current_state() == pb::ConnectionState::Connected {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    if !connected {
        anyhow::bail!("account did not reach Connected state");
    }
    println!("✅ reconnected WITHOUT re-pairing (persisted session works)");

    // Give app-state a moment, then list participating groups. `Client::groups
    // ().get_participating()` is gone on main; `list_participating` reuses the
    // domain helper (#30), which issues the full `GroupParticipatingIq` itself
    // rather than main's slim `list_participating` (no participants/description)
    // -- same reasoning as `src/domain/groups.rs::list_participating`.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let found_groups = groups::list_participating(client.clone()).await?;
    println!("participating in {} groups", found_groups.len());

    let needle_lower = needle.to_lowercase();
    let found = found_groups.iter().find(|g| {
        g.subject.starts_with(&needle) || g.subject.to_lowercase().contains(&needle_lower)
    });

    match found {
        Some(summary) => {
            println!("→ group: \"{}\"  jid={}", summary.subject, summary.jid);
            let jid = jid_parse::parse_jid(&summary.jid)?;
            let result = messaging::send_text(
                &client,
                jid,
                &pb::SendTextRequest {
                    text: text.clone(),
                    ..Default::default()
                },
            )
            .await?;
            println!("✅ sent to group, id={}", result.message_id);
        }
        None => {
            println!("group starting with \"{needle}\" NOT found. Groups I'm in:");
            let mut names: Vec<&String> = found_groups.iter().map(|g| &g.subject).collect();
            names.sort();
            for name in names {
                println!("  - {name}");
            }
        }
    }
    Ok(())
}
