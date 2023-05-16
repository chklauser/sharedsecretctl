use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(kind = "SharedSecret", group = "sharedsecretctl.klauser.link", version = "v1", namespaced)]
#[kube(status = "SharedSecretStatus")]
pub struct SharedSecretSpec {
    pub secret_name: String,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, JsonSchema)]
pub struct SharedSecretStatus {
    pub state: SharedSecretState,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub enum SharedSecretState {
    #[default]
    Uninitialized,
    SecretMissing,
    SecretInvalid,
    Valid,
}

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(kind = "SharedSecretRequest", group = "sharedsecretctl.klauser.link", version = "v1", namespaced)]
#[kube(status = "SharedSecretRequestStatus")]
pub struct SharedSecretRequestSpec {
    pub shared_secret: SharedSecretReference,
    pub local_secret_name: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct SharedSecretReference {
    pub namespace: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, JsonSchema)]
pub struct SharedSecretRequestStatus {
    pub state: SharedSecretRequestState,
    pub last_updated_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Copy, Clone, Default, Debug, Eq, PartialEq, JsonSchema)]
pub enum SharedSecretRequestState {
    #[default]
    Uninitialized,
    SharedSecretMissing,
    SharedSecretInvalid,
    Synchronized,
}
