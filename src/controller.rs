use std::sync::Arc;

use k8s_openapi::api::core::v1::{ObjectReference, Secret};
use kube::{Api, Client};
use kube::api::ListParams;
use kube::runtime::Controller;
use kube::runtime::events::{Recorder, Reporter};
use kube::runtime::reflector::{ObjectRef, Store};
use kube::runtime::watcher::Config;
use tokio_stream::StreamExt as TokioStreamExt;
use tracing::error;

pub use crd::*;

use crate::controller::request::{reconcile_shared_secret_request, shared_secret_request_error_policy};
use crate::controller::shared::{reconcile_shared_secret, shared_secret_error_policy};

mod crd;
mod shared;
mod request;

const CONTROLLER_NAME: &'static str = "sharedsecretctl";

pub struct State {

}

impl State {
    fn to_context(&self, client: Client, reporter: Arc<Reporter>) -> Arc<Context> {
        Arc::new(Context {
            client,
            reporter,
        })
    }
}

#[derive(Clone)]
pub(in crate::controller) struct Context {
    pub client: Client,
    pub reporter: Arc<Reporter>,
}

impl Context {
    pub fn event_recorder(&self, reference: ObjectReference) -> Recorder {
        Recorder::new(self.client.clone(), (*self.reporter).clone(), reference)
    }
}

pub async fn run(state: State) {
    let client = Client::try_default().await.expect("Failed to create kube client");
    let shared_secrets = Api::<SharedSecret>::all(client.clone());
    let secrets = Api::<Secret>::all(client.clone());
    let shared_secret_requests = Api::<SharedSecretRequest>::all(client.clone());
    let reporter = Arc::new(Reporter {
        controller: CONTROLLER_NAME.into(),
        instance: std::env::var("CONTROLLER_POD_NAME").ok(),
    });

    // Verify that we can access the CRD. If we can't, this usually means that
    // the CRD is not installed. (Could also be a permissions issue.)
    if let Err(e) = shared_secrets.list(&ListParams::default().limit(1)).await {
        error!("CRD SharedSecret is not queryable; {e:?}. Is the CRD installed?");
        std::process::exit(1);
    }
    if let Err(e) = shared_secret_requests.list(&ListParams::default().limit(1)).await {
        error!("CRD SharedSecretRequest is not queryable; {e:?}. Is the CRD installed?");
        std::process::exit(1);
    }

    let context = state.to_context(client.clone(), reporter.clone());
    let shared_controller = Controller::new(shared_secrets.clone(), Config::default().any_semantic());
    let shared_store = shared_controller.store();
    let shared_secret_controller = shared_controller
        .shutdown_on_signal()
        .run(reconcile_shared_secret, shared_secret_error_policy, context.clone())
        .map(|_| ());

    let shared_secret_request_controller = futures::StreamExt::boxed({
        let request_controller = Controller::new(
            shared_secret_requests,
            Config::default().any_semantic(),
        );
        let request_store = request_controller.store();
        let request_controller = request_controller
            .shutdown_on_signal()
            .watches(shared_secrets, Config::default().any_semantic(), move |shared_secret| {
                matching_requests(&request_store, &shared_secret)
            });
        let request_store = request_controller.store();
        request_controller
            .watches(secrets.clone(), Config::default().any_semantic(), move |secret| {
                shared_store.find(|shared| {
                    Some(&shared.spec.secret_name[..]) == secret.metadata.name.as_ref().map(|s| &s[..])
                }).and_then(|shared_secret| matching_requests(&request_store, &*shared_secret))
            })
            .owns(secrets, Config::default().any_semantic())
            .run(reconcile_shared_secret_request, shared_secret_request_error_policy, context.clone())
    }        .map(|_| ()));

    futures::StreamExt::for_each(
        shared_secret_controller.merge(shared_secret_request_controller),
        |_| futures::future::ready(()))
        .await;
}

fn matching_requests(request_store: &Store<SharedSecretRequest>, shared_secret: &SharedSecret) -> Option<ObjectRef<SharedSecretRequest>> {
    request_store.find(|request| {
        Some(&request.spec.shared_secret.name[..]) == shared_secret.metadata.name.as_ref().map(|s| &s[..])
            && Some(&request.spec.shared_secret.namespace[..]) == shared_secret.metadata.namespace.as_ref().map(|s| &s[..])
    })
        .map(|request| ObjectRef::from_obj(&*request))
}

