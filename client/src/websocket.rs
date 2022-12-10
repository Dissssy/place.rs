#![allow(dead_code)]
use anyhow::{anyhow, Error};
use async_std::stream::StreamExt;
use futures_util::SinkExt;
use place_rs_shared::messages::{TimeoutType, ToClientMsg, ToServerMsg};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message as TMessage;

#[derive(Debug)]
pub struct Websocket {
    handle: tokio::task::JoinHandle<()>,
    sender: tokio::sync::mpsc::UnboundedSender<(String, TX)>,
    // receiver: tokio::sync::mpsc::UnboundedReceiver<RX>,
    state: Arc<Mutex<WebsocketState>>,
    callbacks: Arc<Mutex<CallbackHandler>>,
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
        let (ttx, mut trx) = tokio::sync::mpsc::unbounded_channel::<(String, TX)>();
        // let (rtx, rrx) = tokio::sync::mpsc::unbounded_channel::<RX>();
        let state = Arc::new(Mutex::new(WebsocketState::Connecting));
        let sstate = state.clone();
        let callbacks = Arc::new(Mutex::new(CallbackHandler::new()));
        let callbacks_one = callbacks.clone();
        let handle = tokio::spawn(async move {
            let callbacks = callbacks_one;
            let (mut websocket, _resp) = match tokio_tungstenite::connect_async(url).await {
                Ok(websocket) => websocket,
                Err(e) => {
                    // let _ = rtx.send(RX::GenericError(format!("Failed to connect: {}", e)));
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
                            (nonce, TX::Message(msg)) => {
                                if let Err(e) = websocket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&msg).unwrap())).await {
                                    // let _ = rtx.send(RX::GenericError(format!("Failed to send message: {}", e)));
                                    sstate.lock().await.set(WebsocketState::Disconnected("Failed to send message".to_string()));
                                    break;
                                }
                            }
                            (nonce, TX::Close) => {
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
                                            ToClientMsg::Heartbeat(nonce) => {
                                                // send a heartbeat back to the websocket
                                                if let Some(nonce) = nonce {
                                                    let _ = callbacks.lock().await.fulfill(nonce.clone(), Callback::Heartbeat(nonce));
                                                }
                                                let _ = websocket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&ToServerMsg::Heartbeat("UwU".to_string())).unwrap())).await;
                                            },
                                            ToClientMsg::GenericError(nonce, e) => {
                                                if let Some(nonce) = nonce {
                                                    let _ = callbacks.lock().await.fulfill(nonce.clone(), Callback::GenericError(nonce, e.clone()));
                                                }
                                                // let _ = rtx.send(RX::GenericError(e));
                                            },
                                            ToClientMsg::TimeoutError(nonce, e) => {
                                                // let _ = rtx.send(RX::TimeoutError(e));
                                            },
                                            _ => {
                                                // let _ = rtx.send(RX::Message(msg));
                                            }
                                        }
                                    },
                                    TMessage::Binary(msg) => {
                                        // let _ = rtx.send(RX::Binary(msg));
                                    },
                                    TMessage::Ping(_) => {
                                        // rtx.send(RX::GenericError("Received ping".to_string())).unwrap(),
                                    },
                                    TMessage::Pong(_) => {
                                        // rtx.send(RX::GenericError("Received pong".to_string())).unwrap(),
                                    },
                                    TMessage::Close(_) => {
                                        sstate.lock().await.set(WebsocketState::Disconnected("Received close".to_string()));
                                        break;
                                    },
                                    TMessage::Frame(_) => {
                                        // rtx.send(RX::GenericError("Received frame".to_string())).unwrap(),
                                    },
                                }
                            }
                            Err(e) => {
                                // let _ = rtx.send(RX::GenericError(format!("Failed to receive message: {}", e)));
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
            // receiver: rrx,
            state,
            callbacks,
        }
    }

    pub async fn send(&mut self, msg: ToServerMsg, nonce: String) -> Result<tokio::sync::oneshot::Receiver<Result<Callback, Error>>, Error> {
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<Callback, Error>>();
        self.callbacks.lock().await.insert(nonce.clone(), tx)?;
        self.sender.send((nonce, TX::Message(msg)))?;
        Ok(rx)
    }

    pub async fn close(&mut self) -> Result<(), Error> {
        self.sender.send(("".to_owned(), TX::Close))?;
        Ok(())
    }

    // pub async fn recv(&mut self) -> Option<RX> {
    //     self.receiver.recv().await
    // }

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

#[derive(Debug, Clone)]
pub enum Callback {
    Heartbeat(String),
    GenericError(String, String),
    TimeoutError(String, String),
    Message(String, ToClientMsg),
    Binary(String, Vec<u8>),
}

#[derive(Debug)]
pub struct CallbackHandler {
    callbacks: HashMap<String, tokio::sync::oneshot::Sender<Result<Callback, Error>>>,
}

impl CallbackHandler {
    // insert a callback into the hashmap
    pub fn insert(&mut self, nonce: String, tx: tokio::sync::oneshot::Sender<Result<Callback, Error>>) -> Result<(), Error> {
        if self.callbacks.insert(nonce, tx).is_some() {
            Err(anyhow!("Duplicate nonce"))
        } else {
            Ok(())
        }
    }
    // attempt to fulfill a callback, return an error if the callback doesn't exist or somehow fails
    pub fn fulfill(&mut self, nonce: String, callback: Callback) -> Result<(), Error> {
        match self.callbacks.remove(&nonce) {
            Some(tx) => {
                tx.send(Ok(callback)).map_err(|_| anyhow!("Failed to send callback"))?;
                Ok(())
            }
            None => Err(anyhow!("Callback doesn't exist")),
        }
    }
    pub fn new() -> Self {
        Self { callbacks: HashMap::new() }
    }
}
