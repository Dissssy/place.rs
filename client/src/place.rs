use std::sync::Arc;

use anyhow::Error;
use place_rs_shared::{hash, messages::ToServerMsg, Color, GenericPixelWithLocation, Place as RawPlace, XY};
use tokio::sync::Mutex;

use crate::websocket::{Callback, Websocket, RX};

pub struct Place {
    place: Arc<Mutex<RawPlace>>,
    websocket: Websocket,
    rest_url: String,
    ws_url: String,
    nonce: u64,
    callback_handler: CallbackHandler,
}

impl Place {
    pub async fn new(ws_url: String, rest_url: String) -> Result<Self, Error> {
        let mut websocket = Websocket::new(ws_url.clone());
        let events = websocket.connect().await?;
        let place = websocket.get_place().await?;
        let place = Arc::new(Mutex::new(place));
        let cplace = place.clone();
        Ok(Self {
            place,
            websocket,
            rest_url,
            ws_url,
            nonce: 0,
            callback_handler: CallbackHandler::new(events, cplace).await,
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
}

struct CallbackHandler {
    handle: tokio::task::JoinHandle<()>,
    sender: tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Receiver<Result<Callback, Error>>>,
}

impl CallbackHandler {
    pub async fn new(events: tokio::sync::mpsc::Receiver<RX>, place: Arc<Mutex<RawPlace>>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(Self::handle_events(events, rx));
        Self { handle, sender: tx }
    }
    async fn handle_events(mut events: tokio::sync::mpsc::Receiver<RX>, mut rx: tokio::sync::mpsc::UnboundedReceiver<tokio::sync::oneshot::Receiver<Result<Callback, Error>>>) {
        loop {
            tokio::select! {
                r = events.recv() => {
                    match r {
                        Some(x) => {
                            println!("Received message: {:?}", x);
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
                            println!("Received message: {:?}", x);
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
