use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use hbb_common::log;
use serde::Serialize;
use serde_derive::Deserialize;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use crate::database::Database;

#[derive(Deserialize)]
pub struct RegisterPeerRequest {
    pub id: String,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub pk: Option<String>,
    #[serde(default)]
    pub info: Option<String>,
}

#[derive(Serialize)]
pub struct PeerResponse {
    pub id: String,
    pub guid: String,
    pub uuid: String,
    pub pk: String,
    pub info: String,
    pub status: Option<i64>,
}

#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            msg: None,
        }
    }

    fn error(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            msg: Some(msg.into()),
        }
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    100
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn to_peer_response(p: crate::database::Peer) -> PeerResponse {
    PeerResponse {
        id: p.id,
        guid: to_hex(&p.guid),
        uuid: to_hex(&p.uuid),
        pk: to_hex(&p.pk),
        info: p.info,
        status: p.status,
    }
}

// POST /api/peers - register a new peer
async fn register_peer(
    Extension(db): Extension<Arc<Database>>,
    Json(req): Json<RegisterPeerRequest>,
) -> impl IntoResponse {
    let id = req.id.trim();
    if id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<PeerResponse>::error("id is required")),
        );
    }

    // Check if peer already exists
    match db.get_peer(id).await {
        Ok(Some(existing)) => {
            let uuid_bytes = req
                .uuid
                .as_deref()
                .map(from_hex)
                .unwrap_or_default();
            let pk_bytes = req
                .pk
                .as_deref()
                .map(from_hex)
                .unwrap_or_default();
            let info_str = req.info.as_deref().unwrap_or(&existing.info);

            if let Err(err) = db
                .update_pk(&existing.guid, id, &pk_bytes, info_str)
                .await
            {
                log::error!("api: update_pk failed: {}", err);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse::<PeerResponse>::error(format!(
                        "db error: {}",
                        err
                    ))),
                );
            }

            // Fetch updated peer
            match db.get_peer(id).await {
                Ok(Some(p)) => (StatusCode::OK, Json(ApiResponse::success(to_peer_response(p)))),
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse::<PeerResponse>::error("failed to fetch updated peer")),
                ),
            }
        }
        Ok(None) => {
            let uuid_bytes = req
                .uuid
                .as_deref()
                .map(from_hex)
                .unwrap_or_else(|| uuid::Uuid::new_v4().as_bytes().to_vec());
            let pk_bytes = req.pk.as_deref().map(from_hex).unwrap_or_default();
            let info_str = req.info.as_deref().unwrap_or("{}");

            match db.insert_peer(id, &uuid_bytes, &pk_bytes, info_str).await {
                Ok(_guid) => match db.get_peer(id).await {
                    Ok(Some(p)) => (
                        StatusCode::CREATED,
                        Json(ApiResponse::success(to_peer_response(p))),
                    ),
                    _ => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse::<PeerResponse>::error(
                            "peer created but failed to fetch",
                        )),
                    ),
                },
                Err(err) => {
                    log::error!("api: insert_peer failed: {}", err);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse::<PeerResponse>::error(format!(
                            "db error: {}",
                            err
                        ))),
                    )
                }
            }
        }
        Err(err) => {
            log::error!("api: get_peer failed: {}", err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<PeerResponse>::error(format!(
                    "db error: {}",
                    err
                ))),
            )
        }
    }
}

// GET /api/peers/:id - get a peer by id
async fn get_peer(
    Extension(db): Extension<Arc<Database>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match db.get_peer(&id).await {
        Ok(Some(p)) => (StatusCode::OK, Json(ApiResponse::success(to_peer_response(p)))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<PeerResponse>::error("peer not found")),
        ),
        Err(err) => {
            log::error!("api: get_peer failed: {}", err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<PeerResponse>::error(format!(
                    "db error: {}",
                    err
                ))),
            )
        }
    }
}

// GET /api/peers - list peers
async fn list_peers(
    Extension(db): Extension<Arc<Database>>,
    Query(query): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = query.limit.clamp(1, 1000);
    let offset = query.offset.max(0);
    match db.get_peers(limit, offset).await {
        Ok(peers) => {
            let list: Vec<PeerResponse> = peers.into_iter().map(to_peer_response).collect();
            (StatusCode::OK, Json(ApiResponse::success(list)))
        }
        Err(err) => {
            log::error!("api: list_peers failed: {}", err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<Vec<PeerResponse>>::error(format!(
                    "db error: {}",
                    err
                ))),
            )
        }
    }
}

// DELETE /api/peers/:id - delete a peer
async fn delete_peer(
    Extension(db): Extension<Arc<Database>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match db.delete_peer(&id).await {
        Ok(affected) => {
            if affected > 0 {
                (
                    StatusCode::OK,
                    Json(ApiResponse::success("peer deleted")),
                )
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse::<&str>::error("peer not found")),
                )
            }
        }
        Err(err) => {
            log::error!("api: delete_peer failed: {}", err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<&str>::error(format!("db error: {}", err))),
            )
        }
    }
}

pub fn build_router(db: Database) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let db = Arc::new(db);
    Router::new()
        .route("/api/peers", get(list_peers).post(register_peer))
        .route("/api/peers/:id", get(get_peer).delete(delete_peer))
        .layer(Extension(db))
        .layer(cors)
}

pub async fn start_api_server(db: Database, port: i32) {
    let router = build_router(db);
    let addr = format!("0.0.0.0:{}", port);
    log::info!("API server listening on http://{}", addr);
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(err) => {
            log::error!("Failed to bind API server on {}: {}", addr, err);
            return;
        }
    };
    if let Err(err) = axum::Server::from_tcp(listener)
        .unwrap()
        .serve(router.into_make_service())
        .await
    {
        log::error!("API server error: {}", err);
    }
}
