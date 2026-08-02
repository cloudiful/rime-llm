use std::sync::Arc;

use anyhow::Result;
use ime_daemon::{
    api,
    config::Config,
    model_client::{ModelClient, ReqwestModelApi},
    session::SessionStore,
};
use pinyin_dict::{Lexicon, UserFreq};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RIME_LLM_DAEMON_LOG").unwrap_or_else(|_| "info".to_string()),
        )
        .with_target(false)
        .compact()
        .init();

    let config = Config::from_env();
    let lexicon = Arc::new(Lexicon::load(&config.dictionary_root));
    let user_freq = Arc::new(Mutex::new(UserFreq::load(&config.user_freq_path)));
    let model = Arc::new(ModelClient::Reqwest(ReqwestModelApi::new(
        config.service_url.clone(),
        config.model_timeout_ms,
    )?));
    let store = Arc::new(SessionStore::new(lexicon, user_freq, config.clone(), model));

    let listener = TcpListener::bind(&config.bind_addr).await?;
    let bind_addr = config.bind_addr.clone();
    tracing::info!(%bind_addr, service_url = %config.service_url, "ime-daemon listening");
    axum::serve(listener, api::router(store))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .expect("install interrupt handler");
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install terminate handler");
    tokio::select! {
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
    }
}
