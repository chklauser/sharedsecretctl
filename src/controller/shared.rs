use std::sync::Arc;
use std::time::Duration;
use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Resource, ResourceExt};
use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use kube::runtime::finalizer::Event;
use serde_json::json;
use tracing::{debug, info, warn};
use crate::controller::{Context, CONTROLLER_NAME, SharedSecret, SharedSecretState, SharedSecretStatus};
use crate::{Error, Result};

static SHARED_SECRET_FINALIZER: &str = "sharedsecretctl.klauser.link/shared-secret";

pub(in crate::controller) async fn reconcile_shared_secret(shared_secret: Arc<SharedSecret>, ctx: Arc<Context>) -> Result<Action> {
    let ns = shared_secret.namespace().unwrap(); // we know that SharedSecret is namespaced
    let shared_secrets = Api::<SharedSecret>::namespaced(ctx.client.clone(), &ns);

    info!("Reconciling SharedSecret \"{}\" in {}", shared_secret.name_any(), ns);
    finalizer(
        &shared_secrets,
        SHARED_SECRET_FINALIZER,
        shared_secret,
        |event| async {
            match event {
                Event::Apply(shared_secret) => shared_secret.apply(ctx.clone()).await,
                Event::Cleanup(shared_secret) => shared_secret.cleanup(ctx.clone()).await,
            }
        }
    ).await
        .map_err(|e| Error::FinalizerError(Box::new(e)))
}

impl SharedSecret {
    async fn apply(&self, ctx: Arc<Context>) -> Result<Action> {
        let ns = &self.meta().namespace.as_ref().unwrap()[..]; // we know that SharedSecret is namespaced
        let client = ctx.client.clone();
        let secrets: Api<Secret> = Api::namespaced(client.clone(), ns);

        let Some(secret) = secrets.get_opt(&self.spec.secret_name).await? else {
            debug!("Secret \"{}.{}\" is missing", self.spec.secret_name, ns);
            self.update_status(&ctx, SharedSecretStatus {
                state: SharedSecretState::SecretMissing,
            }).await?;

            return Ok(Action::requeue(Duration::from_secs(5 * 60)));
        };

        if secret.data.map(|d| d.is_empty()).unwrap_or(true) {
            debug!("Secret \"{}.{}\" is missing", self.spec.secret_name, ns);
            self.update_status(&ctx, SharedSecretStatus {
                state: SharedSecretState::SecretInvalid,
            }).await?;

            return Ok(Action::requeue(Duration::from_secs(5 * 60)));
        }

        self.update_status(&ctx, SharedSecretStatus {
            state: SharedSecretState::Valid,
        }).await?;

        // If no events were received, check back every 5 minutes
        Ok(Action::requeue(Duration::from_secs(5 * 60)))
    }

    async fn cleanup(&self, _ctx: Arc<Context>) -> Result<Action> {

        // If no events were received, check back every 5 minutes
        Ok(Action::requeue(Duration::from_secs(5 * 60)))
    }

    async fn update_status(&self, ctx: &Context, new_status: SharedSecretStatus) -> Result<()> {
        if self.status.as_ref().map(|s| s.update_required(&new_status)) == Some(false) {
            debug!(new_status=?&new_status, "Not updating status of SharedSecret because it is unchanged.");
            return Ok(());
        }

        // we know that SharedSecret is namespaced
        let ns = &self.meta().namespace.as_ref().unwrap()[..];
        let name = &self.metadata.name.as_ref().expect("SharedSecret to have a name")[..];

        let shared_secret_requests: Api<SharedSecret> = Api::namespaced(ctx.client.clone(), ns);
        let new_status_patch = Patch::Apply(json!({
            "apiVersion": "sharedsecretctl.klauser.link/v1",
            "kind": "SharedSecret",
            "status": new_status
        }));
        let ps = PatchParams::apply(CONTROLLER_NAME).force();
        shared_secret_requests.patch_status(name, &ps, &new_status_patch)
            .await
            .map_err(Error::KubeError)?;

        Ok(())
    }
}

impl SharedSecretStatus {
    pub fn update_required(&self, other: &Self) -> bool {
        self.state != other.state
    }
}



pub(in crate::controller) fn shared_secret_error_policy(_doc: Arc<SharedSecret>, error: &Error, _ctx: Arc<Context>) -> Action {
    warn!("SharedSecret reconcile failed: {:?}", error);
    Action::requeue(Duration::from_secs(5 * 60))
}
