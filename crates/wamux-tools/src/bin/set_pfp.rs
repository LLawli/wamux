//! Test SetProfilePicture (square JPEG) + restore on the already-paired,
//! stably-connected account. Usage: set_pfp [socket_path]

use std::io::{Cursor, Read};
use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;

const EXTERNAL_REF: &str = "pair-socket";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/wamux.sock".to_string());
    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut contacts = ContactServiceClient::new(channel);
    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(EXTERNAL_REF.to_string())),
    };

    account
        .connect_account(pb::ConnectAccountRequest {
            account: Some(acct.clone()),
            backfill_history: false,
        })
        .await?;
    let mut own = String::new();
    for _ in 0..100 {
        if let Ok(r) = account.get_account_status(acct.clone()).await {
            let s = r.into_inner();
            if s.state == pb::ConnectionState::Connected as i32 {
                own = s.jid.map(|j| j.value).unwrap_or_default();
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    println!("connected; own={own}");

    let orig_url = contacts
        .get_profile_picture(pb::JidRequest {
            account: Some(acct.clone()),
            jid: own.clone(),
        })
        .await
        .map(|r| r.into_inner().url)
        .unwrap_or_default();
    let orig = if orig_url.is_empty() {
        None
    } else {
        fetch(&orig_url)
    };
    println!(
        "current photo: {}",
        if orig_url.is_empty() {
            "none".into()
        } else {
            format!("present (fetched={})", orig.is_some())
        }
    );

    match contacts
        .set_profile_picture(pb::SetProfilePictureRequest {
            account: Some(acct.clone()),
            image: square_jpeg()?,
        })
        .await
    {
        Ok(_) => println!("✅ SetProfilePicture OK"),
        Err(e) => println!("❌ SetProfilePicture: {} / {}", e.code(), e.message()),
    }
    if let Some(b) = orig {
        let _ = contacts
            .set_profile_picture(pb::SetProfilePictureRequest {
                account: Some(acct.clone()),
                image: b,
            })
            .await;
        println!("restored original");
    } else {
        let _ = contacts.remove_profile_picture(acct.clone()).await;
        println!("restored (removed; had none)");
    }
    Ok(())
}

fn square_jpeg() -> anyhow::Result<Vec<u8>> {
    let mut img = image::RgbImage::new(640, 640);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgb([(x % 256) as u8, (y % 256) as u8, 120]);
    }
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Jpeg)?;
    Ok(buf)
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    let r = ureq::get(url).call().ok()?;
    let mut b = Vec::new();
    r.into_reader()
        .take(16 * 1024 * 1024)
        .read_to_end(&mut b)
        .ok()?;
    Some(b)
}

async fn connect_uds(path: String) -> anyhow::Result<Channel> {
    Ok(Endpoint::try_from("http://[::1]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                Ok::<_, std::io::Error>(TokioIo::new(tokio::net::UnixStream::connect(path).await?))
            }
        }))
        .await?)
}
