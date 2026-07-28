use rime_llm::{api, config::Settings, model::ModelRuntime};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RIME_LLM_LOG").unwrap_or_else(|_| "info".to_string()))
        .with_target(false)
        .compact()
        .init();

    let settings = Settings::load()?;
    tracing::info!(model = %settings.model_repo, file = %settings.model_file, "initializing local Rime model");
    let runtime = ModelRuntime::load(&settings).await?;
    let listener = TcpListener::bind(settings.bind_addr).await?;
    let bind_addr = settings.bind_addr;
    let app = api::router(api::AppState::new(settings, runtime));
    tracing::info!(%bind_addr, "rime-llm listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                .expect("install interrupt handler");
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install terminate handler");
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
