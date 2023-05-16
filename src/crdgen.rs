use kube::CustomResourceExt;

fn main() {
    println!("{}", serde_yaml::to_string(&controller::controller::SharedSecret::crd()).unwrap());
    println!("---");
    println!("{}", serde_yaml::to_string(&controller::controller::SharedSecretRequest::crd()).unwrap());
}