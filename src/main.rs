use std::io;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let collector = tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .json()
        .with_env_filter(EnvFilter::from_default_env())
        .finish();
    collector.init();

    controller::controller::run(controller::controller::State {}).await;

    Ok(())
}
