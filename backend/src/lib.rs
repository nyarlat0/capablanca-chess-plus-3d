mod database;
mod hub;
mod websocket;

use std::{env, io};

use actix_web::{App, HttpResponse, HttpServer, middleware::Logger, web};
use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

use crate::{database::Repository, hub::ConnectionHub, websocket::AppState};

pub async fn run() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let database_url = env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&database_url)
        .await
        .context("failed to connect to PostgreSQL")?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("failed to apply PostgreSQL migrations")?;

    let state = AppState {
        repository: Repository::new(pool.clone()),
        hub: ConnectionHub::default(),
    };
    tracing::info!(%bind_addr, "multiplayer backend listening");
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .wrap(Logger::default())
            .route("/health", web::get().to(health))
            .route("/ws", web::get().to(websocket::websocket))
    })
    .bind(&bind_addr)
    .with_context(|| format!("failed to bind backend to {bind_addr}"))?
    .run()
    .await
    .map_err(io::Error::other)?;

    pool.close().await;
    Ok(())
}

async fn health() -> HttpResponse {
    HttpResponse::Ok().body("ok")
}
