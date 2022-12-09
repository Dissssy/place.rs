#![allow(dead_code)]
use anyhow::{anyhow, Error};
use async_std::stream::StreamExt;
use futures_util::SinkExt;
use place_rs_shared::messages::{TimeoutType, ToClientMsg, ToServerMsg};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message as TMessage;

#[derive(Debug)]
pub struct Websocket {
    handle: tokio::task::JoinHandle<()>,
    sender: tokio::sync::mpsc::UnboundedSender<TX>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<RX>,
    state: Arc<Mutex<WebsocketState>>,
}

#[derive(Debug, Clone)]
pub enum WebsocketState {
    Connecting,
    Connected,
    Disconnected(String),
}

impl WebsocketState {
    fn set(&mut self, state: WebsocketState) {
        *self = state;
    }
}

#[derive(Debug)]
enum TX {
    Message(ToServerMsg),
    Close,
}

#[derive(Debug)]
pub enum RX {
    Message(ToClientMsg),
    Binary(Vec<u8>),
    GenericError(String),
    TimeoutError(TimeoutType),
}

impl Websocket {
    pub fn new(url: String) -> Self {
        let (ttx, mut trx) = tokio::sync::mpsc::unbounded_channel::<TX>();
        let (rtx, rrx) = tokio::sync::mpsc::unbounded_channel::<RX>();
        let state = Arc::new(Mutex::new(WebsocketState::Connecting));
        let sstate = state.clone();
        let handle = tokio::spawn(async move {
            let (mut websocket, _resp) = match tokio_tungstenite::connect_async(url).await {
                Ok(websocket) => websocket,
                Err(e) => {
                    let _ = rtx.send(RX::GenericError(format!("Failed to connect: {}", e)));
                    sstate.lock().await.set(WebsocketState::Disconnected("Failed to connect".to_string()));
                    return;
                }
            };
            let mut state = sstate.lock().await;
            *state = WebsocketState::Connected;
            drop(state);
            loop {
                tokio::select! {
                    Some(msg) = trx.recv() => {
                        match msg {
                            TX::Message(msg) => {
                                if let Err(e) = websocket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&msg).unwrap())).await {
                                    let _ = rtx.send(RX::GenericError(format!("Failed to send message: {}", e)));
                                    sstate.lock().await.set(WebsocketState::Disconnected("Failed to send message".to_string()));
                                    break;
                                }
                            }
                            TX::Close => {
                                break;
                            }
                        }
                    }
                    Some(msg) = websocket.next() => {
                        match msg {
                            Ok(msg) => {
                                match msg {
                                    TMessage::Text(msg) => {
                                        let msg: ToClientMsg = serde_json::from_str(&msg).unwrap();
                                        match msg.clone() {
                                            ToClientMsg::Heartbeat => {
                                                // send a heartbeat back to the websocket
                                                let _ = websocket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&ToServerMsg::Heartbeat).unwrap())).await;
                                            },
                                            ToClientMsg::GenericError(e) => {
                                                let _ = rtx.send(RX::GenericError(e));
                                            },
                                            ToClientMsg::TimeoutError(e) => {
                                                let _ = rtx.send(RX::TimeoutError(e));
                                            },
                                            _ => {
                                                let _ = rtx.send(RX::Message(msg));
                                            }
                                        }
                                    },
                                    TMessage::Binary(msg) => {
                                        let _ = rtx.send(RX::Binary(msg));
                                    },
                                    TMessage::Ping(_) => rtx.send(RX::GenericError("Received ping".to_string())).unwrap(),
                                    TMessage::Pong(_) => rtx.send(RX::GenericError("Received pong".to_string())).unwrap(),
                                    TMessage::Close(_) => {
                                        sstate.lock().await.set(WebsocketState::Disconnected("Received close".to_string()));
                                        break;
                                    }
                                    TMessage::Frame(_) => rtx.send(RX::GenericError("Received frame".to_string())).unwrap(),
                                }
                            }
                            Err(e) => {
                                let _ = rtx.send(RX::GenericError(format!("Failed to receive message: {}", e)));
                                break;
                            }
                        }
                    }
                }
            }
            sstate.lock().await.set(WebsocketState::Disconnected("Disconnected".to_string()));
            drop(sstate);
        });
        Self {
            handle,
            sender: ttx,
            receiver: rrx,
            state,
        }
    }

    pub async fn send(&mut self, msg: ToServerMsg) -> Result<(), Error> {
        self.sender.send(TX::Message(msg))?;
        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), Error> {
        self.sender.send(TX::Close)?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Option<RX> {
        self.receiver.recv().await
    }

    pub async fn state(&self) -> WebsocketState {
        let state = self.state.lock().await;
        state.clone()
    }

    pub async fn connect(&self) -> Result<(), Error> {
        // just waiting for the state to change to connected
        loop {
            let state = self.state.lock().await;
            match state.clone() {
                WebsocketState::Connected => break,
                WebsocketState::Disconnected(e) => return Err(anyhow!(e)),
                _ => {}
            }
            drop(state);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }
}
