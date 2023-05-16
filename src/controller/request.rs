use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::{Api, Resource, ResourceExt};
use kube::api::{Patch, PatchParams, PostParams};
use kube::runtime::finalizer;
use kube::runtime::controller::Action;
use kube::runtime::events::{Event, EventType};
use kube::runtime::finalizer::Event as Finalizer;
use serde_json::json;
use tracing::{debug, info, instrument, warn};

use crate::{Error, Result};
use crate::controller::{Context, CONTROLLER_NAME, SharedSecret, SharedSecretRequest, SharedSecretRequestState, SharedSecretRequestStatus, SharedSecretState};

static SHARED_SECRET_REQUEST_FINALIZER: &str = "sharedsecretctl.klauser.link/shared-secret-request";

pub(in crate::controller) async fn reconcile_shared_secret_request(shared_secret_request: Arc<SharedSecretRequest>, ctx: Arc<Context>) -> Result<Action> {
    let ns = shared_secret_request.namespace().unwrap(); // we know that SharedSecret is namespaced
    let shared_secret_requests = Api::<SharedSecretRequest>::namespaced(ctx.client.clone(), &ns);

    info!("Reconciling SharedSecretRequest \"{}\" in {}", shared_secret_request.name_any(), ns);
    finalizer(
        &shared_secret_requests,
        SHARED_SECRET_REQUEST_FINALIZER,
        shared_secret_request,
        |event| async {
            match event {
                Finalizer::Apply(shared_secret_request) => shared_secret_request.apply(ctx.clone()).await,
                Finalizer::Cleanup(shared_secret_request) => shared_secret_request.cleanup(ctx.clone()).await,
            }
        }
    ).await
        .map_err(|e| Error::FinalizerError(Box::new(e)))
}

impl SharedSecretRequest {
    #[instrument(skip(self, ctx), fields(name = self.metadata.name, namespace = self.metadata.namespace))]
    async fn apply(&self, ctx: Arc<Context>) -> Result<Action> {
        // we know that SharedSecretRequest is namespaced
        let local_ns = &self.meta().namespace.as_ref().unwrap()[..];
        let name = &self.metadata.name.as_ref().expect("SharedSecretRequest to have a name")[..];
        let client = ctx.client.clone();
        let remote_ns = &self.spec.shared_secret.namespace[..];

        let shared_secrets: Api<SharedSecret> = Api::namespaced(client.clone(), remote_ns);
        let remote_secrets: Api<Secret> = Api::namespaced(client.clone(), remote_ns);
        let local_secrets: Api<Secret> = Api::namespaced(client.clone(), local_ns);

        // Check that shared secret exists
        let Some(shared_secret) = shared_secrets.get_opt(&self.spec.shared_secret.name).await? else {
            debug!("SharedSecret \"{}.{}\" is missing", self.spec.shared_secret.name, remote_ns);
            self.update_status(&ctx, SharedSecretRequestStatus {
                state: SharedSecretRequestState::SharedSecretMissing,
                last_updated_at: Some(Utc::now()),
            }).await?;

            return Ok(Action::requeue(Duration::from_secs(5 * 60)));
        };

        // Check that shared secret is valid
        let remote_state = shared_secret.status.map(|s| s.state);
        if remote_state != Some(SharedSecretState::Valid) {
            debug!("SharedSecret \"{}.{}\" is in state {:?}, expecting {:?} instead", self.spec.shared_secret.name, remote_ns, remote_state, SharedSecretState::Valid);
            self.update_status(&ctx, SharedSecretRequestStatus {
                state: SharedSecretRequestState::SharedSecretInvalid,
                last_updated_at: Some(Utc::now()),
            }).await?;

            return Ok(Action::requeue(Duration::from_secs(5 * 60)));
        }

        // TODO: check that this namespace is permitted to read the shared secret

        // Check that the remote secret exists
        let Some(remote_secret) = remote_secrets.get_opt(&shared_secret.spec.secret_name).await? else {
            self.update_status(&ctx, SharedSecretRequestStatus {
                state: SharedSecretRequestState::SharedSecretInvalid,
                last_updated_at: Some(Utc::now()),
            }).await?;

            return Ok(Action::requeue(Duration::from_secs(5 * 60)));
        };

        let events = ctx.event_recorder(self.object_ref(&()));

        // Create or update local secret
        let local_secret_name = self.spec.local_secret_name.as_ref().map(|s| &s[..]).unwrap_or(name);
        match local_secrets.get_opt(local_secret_name).await? {
            None => {
                info!("Local secret \"{}\" for SharedSecretRequest \"{}\" in {} does not exist. Creating...", local_secret_name, name, local_ns);
                let local_secret = Secret {
                    metadata: ObjectMeta {
                        name: Some(local_secret_name.to_string()),
                        namespace: Some(local_ns.to_string()),
                        owner_references: Some(vec![OwnerReference {
                            api_version: "sharedsecretctl.klauser.link/v1".to_string(),
                            kind: "SharedSecretRequest".to_string(),
                            name: name.to_string(),
                            uid: self.metadata.uid.clone().unwrap(),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    },
                    data: remote_secret.data.clone(),
                    ..Default::default()
                };
                let mut ps = PostParams::default();
                ps.field_manager = Some(CONTROLLER_NAME.to_string());
                let created = local_secrets.create(&ps, &local_secret).await?;
                events.publish(Event {
                    action: "Creating".into(),
                    reason: "LocalSecretMissing".into(),
                    note: None,
                    secondary: Some(created.object_ref(&())),
                    type_: EventType::Normal,
                }).await?;
            }
            Some(local_secret) if local_secret.data != remote_secret.data => {
                info!("Local secret \"{}\" for SharedSecretRequest \"{}\" in {} is out of sync. Updating...", local_secret_name, name, local_ns);
                let local_secret_patch = Patch::Merge(Secret {
                    data: remote_secret.data.clone(),
                    ..Default::default()
                });
                let ps = PatchParams::apply(CONTROLLER_NAME);
                let updated = local_secrets.patch(local_secret_name, &ps, &local_secret_patch).await?;
                events.publish(Event {
                    action: "Updating".into(),
                    reason: "LocalSecretOutdated".into(),
                    note: None,
                    secondary: Some(updated.object_ref(&())),
                    type_: EventType::Normal,
                }).await?;
            },
            _ => {
                info!("SharedSecretRequest \"{}\" in {} is still synchronized. Nothing to do.", name, local_ns);
            }
        }

        // Mark ourselves as synchronized
        if self.status.as_ref().map(|s| s.state) != Some(SharedSecretRequestState::Synchronized) {
            self.update_status(&ctx, SharedSecretRequestStatus {
                state: SharedSecretRequestState::Synchronized,
                last_updated_at: Some(Utc::now()),
            }).await?;
        }

        // If no events were received, check back every 5 minutes
        Ok(Action::requeue(Duration::from_secs(5 * 60)))
    }

    async fn update_status(&self, ctx: &Context, new_status: SharedSecretRequestStatus) -> Result<()> {
        if self.status.as_ref().map(|s| s.update_required(&new_status)) == Some(false) {
            debug!(new_status=?&new_status, "Not updating status of SharedSecretRequest because it is unchanged.");
            return Ok(());
        }

        // we know that SharedSecretRequest is namespaced
        let ns = &self.meta().namespace.as_ref().unwrap()[..];
        let name = &self.metadata.name.as_ref().expect("SharedSecretRequest to have a name")[..];

        let shared_secret_requests: Api<SharedSecretRequest> = Api::namespaced(ctx.client.clone(), ns);
        let new_status_patch = Patch::Apply(json!({
        "apiVersion": "sharedsecretctl.klauser.link/v1",
        "kind": "SharedSecretRequest",
        "status": new_status
    }));
        let ps = PatchParams::apply(CONTROLLER_NAME).force();
        shared_secret_requests.patch_status(name, &ps, &new_status_patch)
            .await
            .map_err(Error::KubeError)?;

        Ok(())
    }

    async fn cleanup(&self, ctx: Arc<Context>) -> Result<Action> {
        // we know that SharedSecretRequest is namespaced
        let _ns = &self.meta().namespace.as_ref().unwrap()[..];
        let _name = &self.metadata.name.as_ref().expect("SharedSecretRequest to have a name")[..];
        let _client = ctx.client.clone();

        // If no events were received, check back every 5 minutes
        Ok(Action::requeue(Duration::from_secs(5 * 60)))
    }
}

impl SharedSecretRequestStatus {
    fn update_required(&self, other: &SharedSecretRequestStatus) -> bool {
        self.state != other.state
    }
}

pub(in crate::controller) fn shared_secret_request_error_policy(_doc: Arc<SharedSecretRequest>, error: &Error, _ctx: Arc<Context>) -> Action {
    warn!("SharedSecretRequest reconcile failed: {:?}", error);
    Action::requeue(Duration::from_secs(5 * 60))
}
