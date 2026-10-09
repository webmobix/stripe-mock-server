use stripe_mock::config::Config;
use stripe_mock::{boot, build_router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "stripe_mock=info,tower_http=info".into()),
        )
        .init();

    let path = std::env::var("STRIPE_MOCK_CONFIG").unwrap_or_else(|_| "config.toml".to_string());
    let config = Config::from_path(&path)?;
    let (state, listener) = boot(config).await?;
    tracing::info!("stripe-mock listening on http://{}", listener.local_addr()?);
    axum::serve(listener, build_router(state)).await?;
    Ok(())
}
