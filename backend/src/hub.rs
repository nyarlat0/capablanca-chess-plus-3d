use std::{collections::HashMap, sync::Arc};

use multiplayer_protocol::{ServerMessage, Side};
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub(crate) enum Outbound {
    Message(ServerMessage),
    Close,
}

#[derive(Clone)]
struct PlayerConnection {
    connection_id: Uuid,
    sender: mpsc::UnboundedSender<Outbound>,
}

#[derive(Clone, Default)]
pub(crate) struct ConnectionHub {
    rooms: Arc<RwLock<HashMap<String, HashMap<Side, PlayerConnection>>>>,
}

impl ConnectionHub {
    pub(crate) async fn register(
        &self,
        game_id: &str,
        side: Side,
        connection_id: Uuid,
        sender: mpsc::UnboundedSender<Outbound>,
    ) -> bool {
        let mut rooms = self.rooms.write().await;
        let room = rooms.entry(game_id.to_owned()).or_default();
        let opponent_connected = room.contains_key(&side.opposite());
        if let Some(previous) = room.insert(
            side,
            PlayerConnection {
                connection_id,
                sender,
            },
        ) {
            let _ = previous
                .sender
                .send(Outbound::Message(ServerMessage::Error {
                    code: "connection_replaced".to_owned(),
                    message: "This player reconnected from another client.".to_owned(),
                }));
            let _ = previous.sender.send(Outbound::Close);
        }
        opponent_connected
    }

    pub(crate) async fn unregister(&self, game_id: &str, side: Side, connection_id: Uuid) {
        let mut rooms = self.rooms.write().await;
        let mut remove_room = false;
        if let Some(room) = rooms.get_mut(game_id) {
            if room
                .get(&side)
                .is_some_and(|connection| connection.connection_id == connection_id)
            {
                room.remove(&side);
                if let Some(opponent) = room.get(&side.opposite()) {
                    let _ = opponent.sender.send(Outbound::Message(
                        ServerMessage::OpponentConnection { connected: false },
                    ));
                }
            }
            remove_room = room.is_empty();
        }
        if remove_room {
            rooms.remove(game_id);
        }
    }

    pub(crate) async fn notify_opponent(&self, game_id: &str, side: Side, message: ServerMessage) {
        let rooms = self.rooms.read().await;
        if let Some(opponent) = rooms
            .get(game_id)
            .and_then(|room| room.get(&side.opposite()))
        {
            let _ = opponent.sender.send(Outbound::Message(message));
        }
    }

    pub(crate) async fn broadcast(&self, game_id: &str, message: ServerMessage) {
        let rooms = self.rooms.read().await;
        if let Some(room) = rooms.get(game_id) {
            for connection in room.values() {
                let _ = connection.sender.send(Outbound::Message(message.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reconnect_replaces_only_the_same_color_connection() {
        let hub = ConnectionHub::default();
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let (first_sender, mut first_receiver) = mpsc::unbounded_channel();
        let (second_sender, _second_receiver) = mpsc::unbounded_channel();

        assert!(
            !hub.register("ROOM", Side::White, first_id, first_sender)
                .await
        );
        assert!(
            !hub.register("ROOM", Side::White, second_id, second_sender)
                .await
        );
        assert!(matches!(
            first_receiver.recv().await,
            Some(Outbound::Message(ServerMessage::Error { code, .. }))
                if code == "connection_replaced"
        ));
        assert!(matches!(first_receiver.recv().await, Some(Outbound::Close)));

        // The stale socket must not remove its replacement while shutting down.
        hub.unregister("ROOM", Side::White, first_id).await;
        let (black_sender, _black_receiver) = mpsc::unbounded_channel();
        assert!(
            hub.register("ROOM", Side::Black, Uuid::new_v4(), black_sender)
                .await
        );
    }

    #[tokio::test]
    async fn disconnect_notifies_the_opponent() {
        let hub = ConnectionHub::default();
        let white_id = Uuid::new_v4();
        let black_id = Uuid::new_v4();
        let (white_sender, mut white_receiver) = mpsc::unbounded_channel();
        let (black_sender, _black_receiver) = mpsc::unbounded_channel();
        hub.register("ROOM", Side::White, white_id, white_sender)
            .await;
        hub.register("ROOM", Side::Black, black_id, black_sender)
            .await;

        hub.unregister("ROOM", Side::Black, black_id).await;
        assert!(matches!(
            white_receiver.recv().await,
            Some(Outbound::Message(ServerMessage::OpponentConnection {
                connected: false
            }))
        ));
    }
}
