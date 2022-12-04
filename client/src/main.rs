use std::io;

// hashmap
use std::collections::HashMap;

use anyhow::{anyhow, Error};

use futures_util::SinkExt;
use futures_util::StreamExt;
use image::DynamicImage;
use image::GenericImage;
use image::Rgba;
use place_rs_shared::get_blank_data;
use place_rs_shared::RawWebsocketMessage;
use place_rs_shared::SafeInfo;
use place_rs_shared::WebsocketMessage;
use place_rs_shared::{Color, Pixel, User, XY};
use std::mem;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::protocol::Message;

#[tokio::main]
async fn main() {
    // let mut client = WebSocketHandler::new("wss://place.planetfifty.one/ws/").await.unwrap();
    let mut client = PlaceClient::new().await.unwrap();
    loop {
        // read stdin for Command: <command> <args> <args> ...
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let args = input.split_whitespace().map(|x| x.to_string()).collect();
        let wscommand = TX::parse(args);
        match wscommand {
            Ok(command) => {
                client.websocket_send(command).await.unwrap();
            }
            Err(e) => {
                println!("Error: {}", e);
            }
        }
    }
}

#[derive(Debug, Clone)]
enum TX {
    Paint(XY, Color),
    Nick(String),
}

impl TX {
    fn parse(strs: Vec<String>) -> Result<Self, Error> {
        let mut args = strs.iter();
        let command = args.next().ok_or_else(|| anyhow!("No command"))?;
        match command.as_str() {
            "paint" => {
                // paint <x> <y> <r> <g> <b>
                let x = args.next().ok_or_else(|| anyhow!("No x"))?.parse()?;
                let y = args.next().ok_or_else(|| anyhow!("No y"))?.parse()?;
                let r = args.next().ok_or_else(|| anyhow!("No r"))?.parse()?;
                let g = args.next().ok_or_else(|| anyhow!("No g"))?.parse()?;
                let b = args.next().ok_or_else(|| anyhow!("No b"))?.parse()?;
                Ok(TX::Paint(XY { x, y }, Color { r, g, b }))
            }
            "nick" => {
                // nick <name>
                let name = args.next().ok_or_else(|| anyhow!("No name"))?;
                Ok(TX::Nick(name.to_string()))
            }
            "help" => Err(anyhow!("Commands: \n\tpaint <x> <y> <r> <g> <b> \n\tnick <name>")),
            _ => {
                // unknown command
                Err(anyhow!("Unknown command"))
            }
        }
    }
}

#[derive(Debug)]
enum RX {
    Pixel(Pixel),
    User(User),
}

#[allow(dead_code)]
#[derive(Debug)]
struct WebSocketHandler {
    handle: tokio::task::JoinHandle<()>,
    sender: tokio::sync::mpsc::UnboundedSender<TX>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<RX>,
    state: Arc<Mutex<WebsocketState>>,
}

#[derive(Debug)]
enum WebsocketState {
    Connecting,
    Connected,
    Disconnected(String),
}

#[allow(dead_code)]
impl WebSocketHandler {
    async fn new(url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (mut ws_stream, _) = tokio_tungstenite::connect_async_with_config(url, None).await?;

        let listencommand = RawWebsocketMessage::Listen;
        ws_stream.send(Message::Text(serde_json::to_string(&listencommand).unwrap())).await?;
        let state = Arc::new(Mutex::new(WebsocketState::Connecting));
        let (ctx, mut crx) = unbounded_channel::<TX>();
        let (rtx, rrx) = unbounded_channel::<RX>();
        let mstate = state.clone();
        let handle = tokio::spawn(async move {
            let state = mstate;
            loop {
                // here we will be checking for a message from the server, and sending a message to the server with a configurable interval
                tokio::select! {
                    Some(msg) = crx.recv() => {
                        // send message to server
                        match msg {
                            TX::Paint(xy, color) => {
                                let pixel = RawWebsocketMessage::PixelUpdate { location: xy, color };
                                let msg = serde_json::to_string(&pixel).unwrap();
                                if let Err(e) = ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(msg)).await {
                                    *state.lock().await = WebsocketState::Disconnected(e.to_string());
                                }
                            }
                            TX::Nick(name) => {
                                let user = RawWebsocketMessage::SetUsername(name);
                                let msg = serde_json::to_string(&user).unwrap();
                                if let Err(e) = ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(msg)).await {
                                    *state.lock().await = WebsocketState::Disconnected(e.to_string());
                                }
                            }
                        }
                    }
                    Some(msg) = ws_stream.next() => {
                        if let Ok(msg) = msg {
                            // println!("got message from server: {:?}", msg);
                            let msg = serde_json::from_str::<WebsocketMessage>(&msg.to_string()).unwrap();
                            match msg {
                                WebsocketMessage::Pixel(pixel) => {
                                    println!("Pixel: {:?}", pixel);
                                    rtx.send(RX::Pixel(pixel)).unwrap();
                                }
                                WebsocketMessage::User(user) => {
                                    println!("User: {:?}", user);
                                    rtx.send(RX::User(user)).unwrap();
                                }
                                WebsocketMessage::Heartbeat => {
                                    // send heartbeat back
                                    // println!("Heartbeat");
                                    let heartbeat = RawWebsocketMessage::Heartbeat;
                                    let msg = serde_json::to_string(&heartbeat).unwrap();
                                    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(msg)).await.unwrap();
                                }
                                WebsocketMessage::Listening => {
                                    // println!("Listening");
                                    *state.lock().await = WebsocketState::Connected;
                                }
                                WebsocketMessage::Error(err) => {
                                    println!("Error: {:?}", err);
                                }
                            }
                        }
                    }
                    else => {
                        // println!("connection closed");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            handle,
            sender: ctx,
            receiver: rrx,
            state,
        })
    }
    async fn try_send(&mut self, msg: TX) -> Result<(), WebsocketError> {
        self.sender.send(msg).unwrap();
        // wait for 100ms
        tokio::time::sleep(Duration::from_millis(100)).await;
        let l = &*self.state.lock().await;
        if let WebsocketState::Connected = l {
            Ok(())
        } else {
            Err(WebsocketError::from_state(l))
        }
    }
    async fn connect(&mut self) {
        loop {
            let l = &*self.state.lock().await;
            if let WebsocketState::Connected = l {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn try_send_with_reconnect(&mut self, msg: TX) -> Result<(), Box<dyn std::error::Error>> {
        // if we are not connected, return an error
        {
            let l = &*self.state.lock().await;
            match l {
                WebsocketState::Connected => {}
                _ => return Err(Box::new(WebsocketError::from_state(l))),
            }
        }
        if let Err(e) = self.try_send(msg.clone()).await {
            match e {
                WebsocketError::NotConnected => {
                    // println!("Not yet initialized");
                }
                WebsocketError::Disconnected(_) => {
                    let mut newclient = Self::new("wss://place.planetfifty.one/ws/").await?;
                    newclient.connect().await;
                    newclient.try_send(msg).await.unwrap();
                    mem::swap(self, &mut newclient);
                }
            }
        }
        Ok(())
    }
    async fn try_recieve(&mut self) -> Option<RX> {
        self.receiver.try_recv().ok()
    }
}

#[derive(Debug, Clone)]
enum WebsocketError {
    NotConnected,
    Disconnected(String),
}

impl WebsocketError {
    fn from_state(state: &WebsocketState) -> Self {
        match state {
            WebsocketState::Connecting => Self::NotConnected,
            WebsocketState::Connected => Self::NotConnected,
            WebsocketState::Disconnected(e) => Self::Disconnected(e.to_string()),
        }
    }
}

impl std::error::Error for WebsocketError {}

impl std::fmt::Display for WebsocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => write!(f, "Not connected"),
            Self::Disconnected(e) => write!(f, "Disconnected: {}", e),
        }
    }
}

#[derive(Debug)]
struct PlaceClient {
    client: WebSocketHandler,
    place: Vec<Vec<Pixel>>,
    users: LazyUserMap,
    server_info: SafeInfo,
    chunks_loaded: Vec<Vec<bool>>,
}

impl PlaceClient {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut client = WebSocketHandler::new("wss://place.planetfifty.one/ws/").await?;
        let info = get_info().await?;
        let chunks_loaded = vec![vec![false; info.size.x / info.chunk_size]; info.size.y / info.chunk_size];
        client.connect().await;
        Ok(Self {
            client,
            place: get_blank_data(info.size),
            users: LazyUserMap::new(),
            server_info: info,
            chunks_loaded,
        })
    }
    // async fn send(&mut self, msg: TX) -> Result<(), WebsocketError> {
    //     self.client.try_send(msg).await
    // }
    async fn websocket_send(&mut self, msg: TX) -> Result<(), Box<dyn std::error::Error>> {
        self.client.try_send_with_reconnect(msg).await?;
        Ok(())
    }
    async fn recieve(&mut self) -> Option<RX> {
        self.client.try_recieve().await
    }
    async fn update(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        while let Some(msg) = self.recieve().await {
            match msg {
                RX::Pixel(pixel) => {
                    let location = pixel.location.clone();
                    self.place[location.y][location.x] = pixel;
                }
                RX::User(user) => {
                    self.users.put(user.id.clone(), user);
                }
            }
        }
        Ok(())
    }
    async fn get_user(&self, id: &str) -> Result<User, Error> {
        // check if user is in cache, if not, request it from /api/user/{id}
        self.users.get(id).await
    }
    async fn create_image(&self) -> JoinHandle<Result<DynamicImage, Error>> {
        let place = self.place.clone();
        // let users = self.users.clone();
        let size = (self.server_info.size.x as u32, self.server_info.size.y as u32);
        tokio::spawn(async move {
            let mut img = DynamicImage::new_rgba8(size.0, size.1);
            for row in place.iter() {
                for pixel in row.iter() {
                    // let user = users.get(&pixel.user).await?;
                    let color = pixel.color.clone();
                    img.put_pixel(pixel.location.x as u32, pixel.location.y as u32, Rgba([color.r, color.g, color.b, 255]));
                }
            }
            Ok(img)
        })
    }
}

async fn get_info() -> Result<SafeInfo, Box<dyn std::error::Error>> {
    let resp = reqwest::get("https://place.planetfifty.one/api/info").await?;
    let info: SafeInfo = resp.json().await?;
    Ok(info)
}

#[derive(Debug, Clone)]
struct LazyUserMap {
    map: HashMap<String, User>,
}

impl LazyUserMap {
    fn new() -> Self {
        Self { map: HashMap::new() }
    }
    async fn get(&self, id: &str) -> Result<User, Error> {
        if let Some(user) = self.map.get(id) {
            Ok(user.clone())
        } else {
            let url = format!("https://place.planetfifty.one/api/user/{}", id);
            let resp = reqwest::get(&url).await?;
            let user: User = resp.json().await?;
            Ok(user)
        }
    }
    fn put(&mut self, id: String, user: User) {
        self.map.insert(id, user);
    }
}
