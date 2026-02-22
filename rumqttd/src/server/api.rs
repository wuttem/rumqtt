use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::server::db::Database;
use tracing::info;

#[derive(Clone)]
struct ApiState {
    db: Database,
}

#[derive(Deserialize)]
pub struct CreateUserReq {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct UserRes {
    pub username: String,
}

pub async fn start(listen_addr: std::net::SocketAddr, db: Database) {
    let state = ApiState { db };

    let app = Router::new()
        .route("/users", post(create_user))
        .route("/users/:username", get(get_user).delete(delete_user))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(listen_addr).await.unwrap();
    info!("Starting user management API on {}", listen_addr);
    axum::serve(listener, app).await.unwrap();
}

async fn create_user(
    State(state): State<ApiState>,
    Json(payload): Json<CreateUserReq>,
) -> impl IntoResponse {
    match state.db.add_user(&payload.username, &payload.password).await {
        Ok(_) => StatusCode::CREATED,
        Err(e) => {
            tracing::error!("Failed to create user: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn get_user(
    State(state): State<ApiState>,
    Path(username): Path<String>,
) -> impl IntoResponse {
    match state.db.get_user(&username).await {
        Ok(Some(_)) => (StatusCode::OK, Json(UserRes { username })).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn delete_user(
    State(state): State<ApiState>,
    Path(username): Path<String>,
) -> impl IntoResponse {
    match state.db.delete_user(&username).await {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
