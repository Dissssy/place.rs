use std::sync::Arc;

use anyhow::Error;
use futures_util::stream::FuturesUnordered;
use futures_util::TryStreamExt;
use place_rs_shared::messages::ToClientMsg;
use place_rs_shared::{hash, messages::ToServerMsg, Color, GenericPixelWithLocation, Place as RawPlace, XY};
use tokio::sync::oneshot::Receiver;
use tokio::{io::AsyncWriteExt, sync::Mutex};

use crate::websocket::{Callback, Websocket, RX};

pub struct Place {
    place: Arc<Mutex<RawPlace>>,
    websocket: Websocket,
    rest_url: String,
    ws_url: String,
    nonce: u64,
    callback_handler: CallbackHandler,
    changed: Arc<Mutex<bool>>,
}

impl Place {
    pub async fn new(ws_url: String, rest_url: String) -> Result<Self, Error> {
        let mut websocket = Websocket::new(ws_url.clone());
        let events = websocket.connect().await?;
        let place = websocket.get_place().await?;
        let place = Arc::new(Mutex::new(place));
        let cplace = place.clone();
        let changed = Arc::new(Mutex::new(false));
        let cchanged = changed.clone();
        Ok(Self {
            place,
            websocket,
            rest_url,
            ws_url,
            nonce: 0,
            callback_handler: CallbackHandler::new(events, cplace, cchanged).await,
            changed,
        })
    }
    pub async fn paint(&mut self, location: XY, color: Color) -> Result<(), Error> {
        let nonce = hash(format!("{}", self.nonce));
        self.nonce += 1;
        let msg = ToServerMsg::SetPixel(nonce.clone(), GenericPixelWithLocation { location, color });
        self.callback_handler.send_callback(self.websocket.send(msg, nonce).await?).await?;
        Ok(())
    }
    pub async fn chat(&mut self, msg: String) -> Result<(), Error> {
        let nonce = hash(format!("{}", self.nonce));
        self.nonce += 1;
        let msg = ToServerMsg::ChatMsg(nonce.clone(), msg);
        self.callback_handler.send_callback(self.websocket.send(msg, nonce).await?).await?;
        Ok(())
    }
    pub async fn set_username(&mut self, username: String) -> Result<(), Error> {
        let nonce = hash(format!("{}", self.nonce));
        self.nonce += 1;
        let msg = ToServerMsg::SetName(nonce.clone(), username);
        self.callback_handler.send_callback(self.websocket.send(msg, nonce).await?).await?;
        Ok(())
    }
    pub async fn export_img(&self) -> Result<(), Error> {
        let mut path = dirs::desktop_dir().ok_or_else(|| Error::msg("Failed to get desktop path"))?;
        path.push("place.png");
        let r = self.place.lock().await.get_img(path).await;
        if let Err(e) = r {
            println!("Failed to export image: {}", e);
        }
        Ok(())
    }
    pub async fn changed(&self) -> bool {
        let mut changed = self.changed.lock().await;
        let r = *changed;
        *changed = false;
        r
    }
}

struct CallbackHandler {
    handle: tokio::task::JoinHandle<()>,
    sender: tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Receiver<Result<Callback, Error>>>,
}

impl CallbackHandler {
    pub async fn new(events: tokio::sync::mpsc::Receiver<RX>, place: Arc<Mutex<RawPlace>>, changed: Arc<Mutex<bool>>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(Self::handle_events(events, rx, place, changed));
        Self { handle, sender: tx }
    }
    async fn handle_events(
        mut events: tokio::sync::mpsc::Receiver<RX>,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<tokio::sync::oneshot::Receiver<Result<Callback, Error>>>,
        place: Arc<Mutex<RawPlace>>,
        changed: Arc<Mutex<bool>>,
    ) {
        let mut futures: FuturesUnordered<Receiver<Result<Callback, Error>>> = FuturesUnordered::new();
        loop {
            tokio::select! {
                r = futures.try_next() => {
                        match r {
                            Err(e) => {
                                println!("Error: {}", e);
                            }
                            Ok(None) => {}
                            Ok(Some(Ok(x))) => {
                                // println!("Promise resolved");
                                match x {
                                    Callback::Message(_, RX::Message(ToClientMsg::ChatMsg(_, msg))) => {
                                        println!("{}\t: {}", msg.user_id, msg.msg);
                                    }
                                    _ => {
                                        println!("Received message: {:?}", x);
                                    }
                                }
                            }
                            Ok(Some(Err(e))) => {
                                println!("Websocket error: {}", e);
                            }
                        }
                    }
                r = events.recv() => {
                    match r {
                        Some(x) => {
                            match x {
                                RX::Message(msg) => {
                                    match msg {
                                        place_rs_shared::messages::ToClientMsg::PixelUpdate(_, p) => {
                                            let mut place = place.lock().await;
                                            let r = place.set_pixel(p);
                                            if let Err(e) = r {
                                                println!("Failed to set pixel: {}", e);
                                            } else {
                                                let mut changed = changed.lock().await;
                                                *changed = true;
                                            }
                                        },
                                        place_rs_shared::messages::ToClientMsg::UserUpdate(_, u) => {
                                            let mut place = place.lock().await;
                                            let r = place.set_user(u);
                                            if let Err(e) = r {
                                                println!("Failed to set user: {}", e);
                                            }
                                        },
                                        place_rs_shared::messages::ToClientMsg::Heartbeat(_) => {},
                                        place_rs_shared::messages::ToClientMsg::GenericError(_, e) => {
                                            println!("Received error: {}", e);
                                        },
                                        place_rs_shared::messages::ToClientMsg::TimeoutError(_, e) => {
                                            println!("Received timeout error: {:?}", e);
                                        },
                                        place_rs_shared::messages::ToClientMsg::ChatMsg(_, msg) => {
                                            let user = place.lock().await.get_user(msg.user_id.clone());
                                            if let Some(user) = user {
                                                println!("{}: {}", user.name, msg.msg);
                                            } else {
                                                println!("{}: {}", msg.user_id, msg.msg);
                                            }
                                        },
                                    }
                                },
                                _ => println!("Received event: {:?}", x)
                            }
                        }
                        None => {
                            println!("Websocket closed");
                            break;
                        }
                    }
                }
                r = rx.recv() => {
                    match r {
                        Some(x) => {
                            futures.push(x);
                            // println!("Received message: {:?}", x);
                        }
                        None => {
                            println!("Websocket closed");
                            break;
                        }
                    }
                }
            }
        }
    }
    pub async fn send_callback(&mut self, callback: tokio::sync::oneshot::Receiver<Result<Callback, Error>>) -> Result<(), Error> {
        self.sender.send(callback).map_err(|_| Error::msg("Failed to send callback"))?;
        Ok(())
    }
}
