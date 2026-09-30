//! AccountService: account lifecycle + pairing (QR and code as event streams).

use std::sync::Arc;

use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use whatsapp_rust::pair_code::PairCodeOptions;

use super::account_to_proto;
use crate::proto::v1 as pb;
use crate::proto::v1::account_service_server::AccountService;
use crate::state::{AccountHandle, AccountRegistry};

pub struct AccountSvc {
    registry: Arc<AccountRegistry>,
}

impl AccountSvc {
    pub fn new(registry: Arc<AccountRegistry>) -> Self {
        Self { registry }
    }
}

async fn load_jid(registry: &AccountRegistry, handle: &AccountHandle) -> Option<pb::Jid> {
    // `DeviceStore` is a supertrait of `Backend`, so `load()` is reachable
    // straight off the engine-minted `dyn Backend` — no concrete engine type.
    let backend = registry.storage().device_backend(handle.device_id);
    let device = backend.load().await.ok().flatten()?;
    let jid = device.pn.or(device.lid)?;
    Some(pb::Jid {
        value: jid.to_string(),
    })
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

/// Spawn the (re)connect and forward only pairing updates until paired/error.
fn pairing_stream(
    registry: Arc<AccountRegistry>,
    handle: Arc<AccountHandle>,
    pair_code: Option<PairCodeOptions>,
    skip_history: bool,
) -> ReceiverStream<Result<pb::PairingUpdate, Status>> {
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    // Subscribe before connecting so we never miss the first QR/code.
    let mut events = handle.subscribe();
    tokio::spawn(async move {
        {
            let registry = registry.clone();
            let handle = handle.clone();
            tokio::spawn(async move {
                // skip_history is the relay-pure default; the caller flips it via
                // backfill_history to capture the InitialBootstrap dump that the
                // phone pushes right after linking.
                let _ = registry.connect(&handle, pair_code, skip_history).await;
            });
        }
        loop {
            match events.recv().await {
                Ok(envelope) => {
                    if let Some(pb::event_envelope::Event::Pairing(update)) = envelope.event {
                        let done = matches!(
                            update.event,
                            Some(pb::pairing_update::Event::Paired(_))
                                | Some(pb::pairing_update::Event::Error(_))
                        );
                        if tx.send(Ok(update)).await.is_err() {
                            break;
                        }
                        if done {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
    ReceiverStream::new(rx)
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
        let handle = self.registry.resolve(Some(&reference))?;
        let jid = load_jid(&self.registry, &handle).await;
        Ok(Response::new(status_of(&handle, jid)))
    }

    type PairWithQrStream = ReceiverStream<Result<pb::PairingUpdate, Status>>;

    async fn pair_with_qr(
        &self,
        request: Request<pb::PairWithQrRequest>,
    ) -> Result<Response<Self::PairWithQrStream>, Status> {
        let req = request.into_inner();
        let handle = self.registry.resolve(req.account.as_ref())?;
        Ok(Response::new(pairing_stream(
            self.registry.clone(),
            handle,
            None,
            !req.backfill_history,
        )))
    }

    type PairWithCodeStream = ReceiverStream<Result<pb::PairingUpdate, Status>>;

    async fn pair_with_code(
        &self,
        request: Request<pb::PairWithCodeRequest>,
    ) -> Result<Response<Self::PairWithCodeStream>, Status> {
        let req = request.into_inner();
        let handle = self.registry.resolve(req.account.as_ref())?;
        let options = PairCodeOptions {
            phone_number: req.phone_number,
            custom_code: req.custom_code,
            ..Default::default()
        };
        Ok(Response::new(pairing_stream(
            self.registry.clone(),
            handle,
            Some(options),
            !req.backfill_history,
        )))
    }

    async fn connect_account(
        &self,
        request: Request<pb::ConnectAccountRequest>,
    ) -> Result<Response<pb::AccountStatus>, Status> {
        let req = request.into_inner();
        let handle = self.registry.resolve(req.account.as_ref())?;
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
        let handle = self.registry.resolve(Some(&reference))?;
        self.registry.disconnect(&handle).await;
        Ok(Response::new(status_of(&handle, None)))
    }

    async fn logout(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::Empty>, Status> {
        let reference = request.into_inner();
        let handle = self.registry.resolve(Some(&reference))?;
        // Real server-side unlink (RemoveCompanionDevice IQ); requires a live
        // connection, else FailedPrecondition. Local keys are kept (re-pairable);
        // DeleteAccount wipes state.
        self.registry.logout(&handle).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn delete_account(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::Empty>, Status> {
        let reference = request.into_inner();
        let handle = self.registry.resolve(Some(&reference))?;
        self.registry.delete(&handle).await?;
        Ok(Response::new(pb::Empty {}))
    }
}
