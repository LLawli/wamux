//! Resolve a number to its canonical JID via usync (CheckOnWhatsApp) and send a
//! probe text to: (a) each resolved JID, and (b) the @c.us legacy forms. Tells us
//! definitively whether the number is on WhatsApp and which JID actually delivers.
//!
//! Usage: whois [socket_path]   (default /tmp/wamux.sock)

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

const EXTERNAL_REF: &str = "pair-socket";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/wamux.sock".to_string());
    // Edge picks the namespace: send full JIDs to CheckOnWhatsApp (the core
    // no longer guesses/normalizes).
    let candidates = [
        "5562998877474@s.whatsapp.net".to_string(),
        "5511999999999@s.whatsapp.net".to_string(),
    ];

    let channel = connect_uds(socket_path).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut contacts = ContactServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel);
    let account_ref = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(EXTERNAL_REF.to_string())),
    };

    account
        .connect_account(pb::ConnectAccountRequest {
            account: Some(account_ref.clone()),
            backfill_history: false,
        })
        .await?;
    for _ in 0..100 {
        let status = account
            .get_account_status(account_ref.clone())
            .await?
            .into_inner();
        if status.state == pb::ConnectionState::Connected as i32 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    // 1) usync resolution.
    println!("CheckOnWhatsApp for {candidates:?} ...");
    let resp = contacts
        .check_on_whats_app(pb::CheckOnWhatsAppRequest {
            account: Some(account_ref.clone()),
            jids: candidates.to_vec(),
        })
        .await?
        .into_inner();
    if resp.results.is_empty() {
        println!("  (no usync results returned)");
    }
    for r in &resp.results {
        println!(
            "  on_whatsapp={} jid={} (query={})",
            r.is_on_whatsapp, r.jid, r.query
        );
    }

    // 2) send to each resolved JID that is on WhatsApp.
    for r in &resp.results {
        if r.is_on_whatsapp && !r.jid.is_empty() {
            probe(
                &mut messaging,
                &account_ref,
                &r.jid,
                "wamux: via JID resolvido (usync) ✅",
            )
            .await;
        }
    }

    // 3) send to the @c.us legacy forms (as requested).
    for form in ["5562998877474@c.us", "5511999999999@c.us"] {
        probe(&mut messaging, &account_ref, form, "wamux: via c.us ✅").await;
    }

    Ok(())
}

async fn probe(
    messaging: &mut MessagingServiceClient<Channel>,
    account: &pb::AccountRef,
    to: &str,
    text: &str,
) {
    let request = pb::SendTextRequest {
        account: Some(account.clone()),
        to: Some(pb::Jid {
            value: to.to_string(),
        }),
        text: text.to_string(),
        ..Default::default()
    };
    match messaging.send_text(request).await {
        Ok(resp) => {
            let id = resp.into_inner().key.map(|k| k.id).unwrap_or_default();
            println!("[send -> {to}] OK id={id}");
        }
        Err(status) => println!(
            "[send -> {to}] ERR {:?}: {}",
            status.code(),
            status.message()
        ),
    }
}

async fn connect_uds(path: String) -> anyhow::Result<Channel> {
    let channel = Endpoint::try_from("http://[::1]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(TokioIo::new(stream))
            }
        }))
        .await?;
    Ok(channel)
}
