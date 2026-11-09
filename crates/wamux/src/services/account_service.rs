//! AccountService: account lifecycle + pairing (QR and code as event streams).

use std::sync::Arc;

use tokio::sync::broadcast;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use wamux_types::{AccountRef, relay_lib_jid};
use whatsapp_rust::pair_code::PairCodeOptions;

use super::account_to_proto;
use crate::domain::pairing_update::pairing_stream_ends;
use crate::error::WamuxError;
use crate::proto::v1 as pb;
use crate::proto::v1::account_service_server::AccountService;
use crate::state::{AccountHandle, AccountRegistry};
use crate::transport::shutdown::Shutdown;

pub struct AccountSvc {
    registry: Arc<AccountRegistry>,
    shutdown: Shutdown,
}

impl AccountSvc {
    /// A service whose pairing streams end only when their flow finishes or
    /// their client hangs up. What the in-process tests want; the daemon wires
    /// `with_shutdown`.
    pub fn new(registry: Arc<AccountRegistry>) -> Self {
        Self {
            registry,
            shutdown: Shutdown::new(),
        }
    }

    /// End every pairing stream when `shutdown` fires (#247): a stream waiting
    /// on a phone never ends on its own, and tonic waits for it before `serve`
    /// returns.
    pub fn with_shutdown(mut self, shutdown: Shutdown) -> Self {
        self.shutdown = shutdown;
        self
    }
}

async fn load_jid(registry: &AccountRegistry, handle: &AccountHandle) -> Option<pb::Jid> {
    // `DeviceStore` is a supertrait of `Backend`, so `load()` is reachable
    // straight off the engine-minted `dyn Backend` — no concrete engine type.
    let backend = registry.storage().device_backend(handle.device_id);
    let device = backend.load().await.ok().flatten()?;
    let jid = device.pn.or(device.lid)?;
    relay_lib_jid(&jid)
}

fn status_of(handle: &AccountHandle, jid: Option<pb::Jid>) -> pb::AccountStatus {
    pb::AccountStatus {
        uuid: handle.uuid.to_string(),
        state: handle.current_state() as i32,
        jid,
        push_name: handle.push_name.clone().unwrap_or_default(),
        paired_at: 0,
    }
}

type PairingTx = tokio::sync::mpsc::Sender<Result<pb::PairingUpdate, Status>>;

/// Refuse an account holding a paired identity, then connect and forward only
/// pairing updates until paired/error, the client hanging up, or `shutdown`.
/// The library logs in whenever a `pn` is stored, so such an account never
/// asks for a QR and the stream would stay silent (#253); the refusal comes
/// before subscribe and connect so a refused call starts nothing.
///
/// A failed connect fails the call itself, before any stream exists (#250): no
/// pairing event follows a connect that failed, so a stream would stay silent,
/// and the connect only builds the bot, so waiting for it costs milliseconds.
async fn pairing_stream(
    registry: Arc<AccountRegistry>,
    handle: Arc<AccountHandle>,
    pair_code: Option<PairCodeOptions>,
    skip_history: bool,
    shutdown: Shutdown,
) -> Result<ReceiverStream<Result<pb::PairingUpdate, Status>>, Status> {
    if registry.has_stored_pn(&handle).await? {
        return Err(WamuxError::FailedPrecondition(
            "account holds a paired identity: DeleteAccount and CreateAccount (same external_ref) to pair again"
                .to_string(),
        )
        .into());
    }
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    // Subscribe before connecting so we never miss the first QR/code.
    let events = handle.subscribe();
    // skip_history is the relay-pure default; the caller flips it via
    // backfill_history to capture the InitialBootstrap dump that the phone
    // pushes right after linking.
    registry.connect(&handle, pair_code, skip_history).await?;
    tokio::spawn(forward_pairing_updates(events, tx, shutdown));
    Ok(ReceiverStream::new(rx))
}

/// Owns `tx`, so returning drops it, which is the client's clean end of stream.
async fn forward_pairing_updates(
    mut events: broadcast::Receiver<pb::EventEnvelope>,
    tx: PairingTx,
    shutdown: Shutdown,
) {
    // One select around the whole loop, not per `recv`, so a `send` blocked on a
    // client that stopped reading is cut too. `shutdown`: a stream waiting on a
    // phone never ends on its own, and tonic waits for it (#247, see #35).
    // `tx.closed()`: `send` notices a gone client only on the next pairing
    // update, which may never come, and the task would stay parked on the
    // account's broadcast.
    tokio::select! {
        () = relay_pairing_events(&mut events, &tx) => {}
        () = tx.closed() => {}
        () = shutdown.triggered() => {}
    }
}

async fn relay_pairing_events(events: &mut broadcast::Receiver<pb::EventEnvelope>, tx: &PairingTx) {
    loop {
        match events.recv().await {
            Ok(envelope) => {
                let Some(pb::event_envelope::Event::Pairing(update)) = envelope.event else {
                    continue;
                };
                let done = pairing_stream_ends(&update);
                if tx.send(Ok(update)).await.is_err() || done {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => return,
        }
    }
}

#[tonic::async_trait]
impl AccountService for AccountSvc {
    async fn create_account(
        &self,
        request: Request<pb::CreateAccountRequest>,
    ) -> Result<Response<pb::Account>, Status> {
        let req = request.into_inner();
        let handle = self
            .registry
            .create_account(req.external_ref.as_deref())
            .await?;
        Ok(Response::new(account_to_proto(&handle)))
    }

    async fn list_accounts(
        &self,
        _request: Request<pb::ListAccountsRequest>,
    ) -> Result<Response<pb::ListAccountsResponse>, Status> {
        let accounts = self
            .registry
            .list()
            .iter()
            .map(|h| account_to_proto(h))
            .collect();
        Ok(Response::new(pb::ListAccountsResponse { accounts }))
    }

    async fn get_account_status(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::AccountStatus>, Status> {
        let reference = request.into_inner();
        let handle = self
            .registry
            .resolve(&AccountRef::from_proto(Some(&reference))?)?;
        let jid = load_jid(&self.registry, &handle).await;
        Ok(Response::new(status_of(&handle, jid)))
    }

    type PairWithQrStream = ReceiverStream<Result<pb::PairingUpdate, Status>>;

    async fn pair_with_qr(
        &self,
        request: Request<pb::PairWithQrRequest>,
    ) -> Result<Response<Self::PairWithQrStream>, Status> {
        let req = request.into_inner();
        let handle = self
            .registry
            .resolve(&AccountRef::from_proto(req.account.as_ref())?)?;
        Ok(Response::new(
            pairing_stream(
                self.registry.clone(),
                handle,
                None,
                !req.backfill_history,
                self.shutdown.clone(),
            )
            .await?,
        ))
    }

    type PairWithCodeStream = ReceiverStream<Result<pb::PairingUpdate, Status>>;

    async fn pair_with_code(
        &self,
        request: Request<pb::PairWithCodeRequest>,
    ) -> Result<Response<Self::PairWithCodeStream>, Status> {
        let req = request.into_inner();
        let handle = self
            .registry
            .resolve(&AccountRef::from_proto(req.account.as_ref())?)?;
        let options = PairCodeOptions {
            phone_number: req.phone_number,
            custom_code: req.custom_code,
            ..Default::default()
        };
        Ok(Response::new(
            pairing_stream(
                self.registry.clone(),
                handle,
                Some(options),
                !req.backfill_history,
                self.shutdown.clone(),
            )
            .await?,
        ))
    }

    async fn connect_account(
        &self,
        request: Request<pb::ConnectAccountRequest>,
    ) -> Result<Response<pb::AccountStatus>, Status> {
        let req = request.into_inner();
        let handle = self
            .registry
            .resolve(&AccountRef::from_proto(req.account.as_ref())?)?;
        self.registry
            .connect(&handle, None, !req.backfill_history)
            .await?;
        Ok(Response::new(status_of(&handle, None)))
    }

    async fn disconnect_account(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::AccountStatus>, Status> {
        let reference = request.into_inner();
        let handle = self
            .registry
            .resolve(&AccountRef::from_proto(Some(&reference))?)?;
        self.registry.disconnect(&handle).await;
        Ok(Response::new(status_of(&handle, None)))
    }

    async fn logout(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::Empty>, Status> {
        let reference = request.into_inner();
        let handle = self
            .registry
            .resolve(&AccountRef::from_proto(Some(&reference))?)?;
        // Real server-side unlink (RemoveCompanionDevice IQ); requires a live
        // connection, else FailedPrecondition. The stored identity stays, so
        // this account cannot pair again: that takes DeleteAccount and
        // CreateAccount.
        self.registry.logout(&handle).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn delete_account(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::Empty>, Status> {
        let reference = request.into_inner();
        let handle = self
            .registry
            .resolve(&AccountRef::from_proto(Some(&reference))?)?;
        self.registry.delete(&handle).await?;
        Ok(Response::new(pb::Empty {}))
    }
}

#[cfg(test)]
#[path = "account_service_tests.rs"]
mod tests;
