use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

const MAX_AUDITED_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone, Default)]
pub struct AppState {
    created_tasks: Arc<AtomicUsize>,
}

impl AppState {
    pub fn created_task_count(&self) -> usize {
        self.created_tasks.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Deserialize)]
struct CreateTask {
    title: String,
}

#[derive(Debug, Serialize)]
struct CreatedTask {
    id: usize,
    title: String,
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/tasks", post(create_task))
        .layer(middleware::from_fn(audit_json_body))
        .with_state(state)
}

async fn audit_json_body(request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();

    let bytes = match to_bytes(body, MAX_AUDITED_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("[audit] リクエストボディの取得に失敗しました: {error}");
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
    };

    eprintln!(
        "[audit] path={} body_bytes={} を監査しました",
        parts.uri.path(),
        bytes.len()
    );

    next.run(Request::from_parts(parts, Body::empty())).await
}

async fn create_task(
    State(state): State<AppState>,
    Json(payload): Json<CreateTask>,
) -> (StatusCode, Json<CreatedTask>) {
    let id = state.created_tasks.fetch_add(1, Ordering::SeqCst) + 1;
    eprintln!("[handler] task_id={id} title={} を作成しました", payload.title);

    (
        StatusCode::CREATED,
        Json(CreatedTask {
            id,
            title: payload.title,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header::CONTENT_TYPE, Request},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn json_post_must_reach_handler_after_audit_middleware() {
        let state = AppState::default();
        let response = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tasks")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"請求書を確認する"}"#))
                    .expect("テスト用リクエストを構築できませんでした"),
            )
            .await
            .expect("ルーターが応答を返しませんでした");

        let status = response.status();
        let created_task_count = state.created_task_count();
        assert!(
            status == StatusCode::CREATED && created_task_count == 1,
            "監査済みのJSON POSTは201と作成済み件数1を返す必要があります: status={status}, created_task_count={created_task_count}"
        );
    }

    #[tokio::test]
    async fn syntactically_invalid_json_is_rejected_without_state_change() {
        let state = AppState::default();
        let response = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tasks")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title": }"#))
                    .expect("テスト用リクエストを構築できませんでした"),
            )
            .await
            .expect("ルーターが応答を返しませんでした");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(state.created_task_count(), 0);
    }
}
