use axum::{
    extract::{ws::{WebSocketUpgrade, Message, WebSocket}, Path, State},
    response::IntoResponse,
};

use tokio::sync::broadcast;
use crate::state::AppState;

pub async fn ws_handler(
    Path(bin_id): Path<String>,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, bin_id, state))
}

async fn handle_socket(mut socket: WebSocket, bin_id: String, state: AppState) {
    let sender = state
        .bin_channels
        .entry(bin_id.clone())
        .or_insert_with(|| {
            let (tx, _) = broadcast::channel(100);
            tx
        })
        .clone();

    let mut receiver = sender.subscribe();

    while let Ok(msg) = receiver.recv().await {
        if socket.send(Message::Text(msg)).await.is_err() {
            break;
        }
    }
}
