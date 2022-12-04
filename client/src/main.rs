use std::fmt::Display;
use std::io;

use std::env;
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use futures_util::SinkExt;
use futures_util::{future, pin_mut, StreamExt};
use place_rs_shared::RawWebsocketMessage;
use place_rs_shared::WebsocketMessage;
use place_rs_shared::{Color, Pixel, User, XY};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::{net::TcpStream, sync::mpsc::unbounded_channel};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tokio_tungstenite::{tungstenite::protocol::WebSocketConfig, MaybeTlsStream, WebSocketStream};

#[tokio::main]
async fn main() {
    let mut client = WebSocketHandler::new("wss://place.planetfifty.one/ws/").await.unwrap();
    loop {
        // read stdin for Command: <command> <args> <args> ...
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let mut args = input.split_whitespace();
        let command = args.next().unwrap();
        let wscommand = match command {
            "paint" => {
                // paint <x> <y> <r> <g> <b>
                let x = args.next().unwrap().parse::<usize>().unwrap();
                let y = args.next().unwrap().parse::<usize>().unwrap();
                let r = args.next().unwrap().parse::<u8>().unwrap();
                let g = args.next().unwrap().parse::<u8>().unwrap();
                let b = args.next().unwrap().parse::<u8>().unwrap();
                TX::Paint(XY { x, y }, Color { r, g, b })
            }
            "nick" => {
                // nick <name>
                let name = args.next().unwrap();
                TX::Nick(name.to_string())
            }
            "help" => {
                println!("Commands: \n\tpaint <x> <y> <r> <g> <b> \n\tnick <name>");
                continue;
            }
            _ => {
                // unknown command
                println!("unknown command");
                continue;
            }
        };
        client = client.try_send_with_reconnect(wscommand).await.unwrap();
    }
}

#[derive(Debug, Clone)]
enum TX {
    Paint(XY, Color),
    Nick(String),
}

#[derive(Debug)]
enum RX {
    Pixel(Pixel),
    User(User),
}

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
    async fn wait_for_reconnect(&mut self) {
        loop {
            let l = &*self.state.lock().await;
            if let WebsocketState::Connected = l {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn try_send_with_reconnect(self, msg: TX) -> Result<Self, Box<dyn std::error::Error>> {
        let mut h = self;
        if let Err(e) = h.try_send(msg.clone()).await {
            match e {
                WebsocketError::NotConnected => {
                    // println!("Not yet initialized");
                }
                WebsocketError::Disconnected(e) => {
                    // println!("Send error: {}", e);
                    // println!("Reconnecting...");
                    let mut newclient = WebSocketHandler::new("wss://place.planetfifty.one/ws/").await?;
                    // println!("Sending command again...");
                    // wait for reconnection
                    newclient.wait_for_reconnect().await;
                    newclient.try_send(msg).await.unwrap();
                    h = newclient;
                    // mem::swap(&mut client, &mut newclient);
                    // drop(newclient);
                }
            }
        }
        Ok(h)
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
