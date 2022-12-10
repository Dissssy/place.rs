use std::sync::Arc;

use anyhow::{anyhow, Error};
use async_std::stream::StreamExt;
use eframe::epaint::ColorImage;
use egui_extras::RetainedImage;
use futures_util::stream::FuturesUnordered;
use place_rs_shared::messages::{SafeInfo, ToClientMsg};
use place_rs_shared::{hash, messages::ToServerMsg, Color, GenericPixelWithLocation, Place as RawPlace, XY};
use place_rs_shared::{ChatMsg, MaybePixel, User};
use poll_promise::Promise;
use tokio::sync::oneshot::Receiver;
use tokio::sync::Mutex;

use crate::websocket::{Callback, Websocket, RX};

pub struct Place {
    place: Arc<Mutex<RawPlace>>,
    websocket: Websocket,
    rest_url: String,
    ws_url: String,
    nonce: u64,
    callback_handler: CallbackHandler,
    changed: Arc<Mutex<bool>>,
    chat_rx: tokio::sync::mpsc::UnboundedReceiver<VerifiedChatMsg>,
}

impl Place {
    pub async fn new(ws_url: String, rest_url: String) -> Result<Self, Error> {
        let mut websocket = Websocket::new(ws_url.clone());
        let events = websocket.connect().await?;
        let place = websocket.get_place().await?;
        let place = Arc::new(Mutex::new(place));
        let cplace = place.clone();
        let changed = Arc::new(Mutex::new(true));
        let cchanged = changed.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Ok(Self {
            place,
            websocket,
            rest_url,
            ws_url,
            nonce: 0,
            callback_handler: CallbackHandler::new(events, cplace, cchanged, tx).await,
            changed,
            chat_rx: rx,
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
        let mut path = dirs::download_dir().ok_or_else(|| Error::msg("Failed to get download path"))?;
        path.push("place.png");
        let r = self.place.lock().await.get_img(path).await;
        if let Err(e) = r {
            println!("Failed to export image: {}", e);
        }
        Ok(())
    }
    pub fn changed(&self) -> bool {
        let changed = self.changed.try_lock();
        match changed {
            Ok(mut changed) => {
                let r = *changed;
                *changed = false;
                r
            }
            Err(_) => false,
        }
    }
    pub fn try_get_msg(&mut self) -> Option<VerifiedChatMsg> {
        self.chat_rx.try_recv().ok()
    }
    pub async fn get_image(place: Arc<Mutex<RawPlace>>, changed: Arc<Mutex<bool>>) -> Result<RetainedImage, Error> {
        let image = {
            let place = place.lock().await;
            place.data.clone()
        }
        .iter()
        .map(|p| {
            p.iter()
                .map(|u| match u.pixel.clone() {
                    MaybePixel::None => Color::default(),
                    MaybePixel::Pixel(x) => x.color,
                })
                .collect::<Vec<Color>>()
        })
        .collect::<Vec<Vec<Color>>>();
        let size = [image.len(), image.get(0).ok_or_else(|| anyhow!("Empty image"))?.len()];
        // now we have a 2d array of colors
        // we need to make it into a 2d array of u8s
        let image = image
            .iter()
            .flat_map(|row| {
                row.iter()
                    .flat_map(|color| {
                        let color = vec![color.r, color.g, color.b, 255u8];
                        color
                    })
                    .collect::<Vec<u8>>()
            })
            .collect::<Vec<u8>>();

        let image = ColorImage::from_rgba_unmultiplied(size, &image[..]);
        let mut changed = changed.lock().await;
        *changed = false;
        Ok(RetainedImage::from_color_image("Place", image))
    }
    pub fn get_image_promise(&self) -> Promise<Result<RetainedImage, Error>> {
        Promise::spawn_async(Self::get_image(self.place.clone(), self.changed.clone()))
    }
    pub async fn get_image_async(&self) -> Result<RetainedImage, Error> {
        Self::get_image(self.place.clone(), self.changed.clone()).await
    }
    pub async fn get_info(&self) -> Result<SafeInfo, Error> {
        // get SafeInfo from the /info REST endpoint
        let url = format!("{}/api/info", self.rest_url);
        let resp = reqwest::get(&url).await?;
        println!("{}", resp.text().await?);
        // let info: SafeInfo = resp.json().await?;
        // Ok(info)
        todo!()
    }
}

struct CallbackHandler {
    handle: tokio::task::JoinHandle<()>,
    sender: tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Receiver<Result<Callback, Error>>>,
}

pub struct VerifiedChatMsg {
    pub msg: String,
    pub user: User,
}

impl CallbackHandler {
    pub async fn new(events: tokio::sync::mpsc::Receiver<RX>, place: Arc<Mutex<RawPlace>>, changed: Arc<Mutex<bool>>, chat_messages: tokio::sync::mpsc::UnboundedSender<VerifiedChatMsg>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(Self::handle_events(events, rx, place, changed, chat_messages));
        Self { handle, sender: tx }
    }
    async fn handle_events(
        mut events: tokio::sync::mpsc::Receiver<RX>,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<tokio::sync::oneshot::Receiver<Result<Callback, Error>>>,
        place: Arc<Mutex<RawPlace>>,
        changed: Arc<Mutex<bool>>,
        chat_messages: tokio::sync::mpsc::UnboundedSender<VerifiedChatMsg>,
    ) {
        let mut futures: FuturesUnordered<Receiver<Result<Callback, Error>>> = FuturesUnordered::new();
        loop {
            // println!("Waiting for events");
            tokio::select! {
                Some(r) = futures.next() => {
                        match r {
                            Err(e) => {
                                println!("Error: {}", e);
                            }
                            Ok(Ok(x)) => {
                                // println!("Promise resolved");
                                match x {
                                    Callback::Message(_, RX::Message(msg)) => {
                                        match msg {
                                            ToClientMsg::ChatMsg(_, msg) => {
                                                let user = place.lock().await.get_user(msg.user_id.clone());
                                                if let Some(user) = user {
                                                    let r = chat_messages.send(VerifiedChatMsg { msg: msg.msg, user });
                                                    if let Err(e) = r {
                                                        println!("Failed to send chat message: {}", e);
                                                    }
                                                } else {
                                                    // println!("{}: {}", msg.user_id, msg.msg);
                                                    println!("User not found: {}", msg.user_id);
                                                }
                                            }
                                            ToClientMsg::PixelUpdate(_, p) => {
                                                let mut place = place.lock().await;
                                                let r = place.set_pixel(p);
                                                if let Err(e) = r {
                                                    println!("Failed to set pixel: {}", e);
                                                } else {
                                                    let mut changed = changed.lock().await;
                                                    *changed = true;
                                                }
                                            }
                                            ToClientMsg::UserUpdate(_, u) => {
                                                let mut place = place.lock().await;
                                                let r = place.set_user(u);
                                                if let Err(e) = r {
                                                    println!("Failed to set user: {}", e);
                                                } else {
                                                    let mut changed = changed.lock().await;
                                                    *changed = true;
                                                }
                                            }
                                            _ => {
                                                // println!("Unhandled message: {:?}", msg);
                                            }
                                        }
                                    }
                                    _ => {
                                        println!("Unhandled message: {:?}", x);
                                    }
                                }
                            }
                            Ok(Err(e)) => {
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
                                        ToClientMsg::PixelUpdate(_, p) => {
                                            let mut place = place.lock().await;
                                            let r = place.set_pixel(p);
                                            if let Err(e) = r {
                                                println!("Failed to set pixel: {}", e);
                                            } else {
                                                let mut changed = changed.lock().await;
                                                *changed = true;
                                            }
                                        },
                                        ToClientMsg::UserUpdate(_, u) => {
                                            let mut place = place.lock().await;
                                            let r = place.set_user(u);
                                            if let Err(e) = r {
                                                println!("Failed to set user: {}", e);
                                            }
                                        },
                                        ToClientMsg::Heartbeat(_) => {},
                                        ToClientMsg::GenericError(_, e) => {
                                            println!("Received error: {}", e);
                                        },
                                        ToClientMsg::TimeoutError(_, e) => {
                                            println!("Received timeout error: {:?}", e);
                                        },
                                        ToClientMsg::ChatMsg(_, msg) => {
                                            let user = place.lock().await.get_user(msg.user_id.clone());
                                            if let Some(user) = user {
                                                let r = chat_messages.send(VerifiedChatMsg { msg: msg.msg, user });
                                                if let Err(e) = r {
                                                    println!("Failed to send chat message: {}", e);
                                                }
                                            } else {
                                                // println!("{}: {}", msg.user_id, msg.msg);
                                                println!("User not found: {}", msg.user_id);
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
            // tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }
    pub async fn send_callback(&mut self, callback: tokio::sync::oneshot::Receiver<Result<Callback, Error>>) -> Result<(), Error> {
        self.sender.send(callback).map_err(|_| Error::msg("Failed to send callback"))?;
        Ok(())
    }
}
